//! A small, borderless, click-through popup that shows briefly (like a
//! volume/caps-lock OSD, or the little IME language indicator) and closes
//! itself -- not a Windows Action Center toast, which is more visible than
//! wanted for something that fires every time a hotkey is pressed and
//! which the user explicitly asked not to use.
//!
//! Each call to `show` spawns its own short-lived thread that creates one
//! popup window, times it out, and exits -- there is never more than a
//! trivial number of these alive at once (one per recent hotkey press), so
//! a dedicated thread per popup is simpler than keeping one window alive
//! and re-showing it, at negligible cost.

use std::sync::Once;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
    GetMonitorInfoW, MonitorFromWindow, SelectObject, SetBkMode, SetTextColor, DT_CENTER,
    DT_SINGLELINE, DT_VCENTER, FW_BOLD, MONITORINFO, MONITOR_DEFAULTTOPRIMARY, PAINTSTRUCT,
    TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
    GetForegroundWindow, GetMessageW, GetWindowLongPtrW, KillTimer, PostQuitMessage,
    RegisterClassExW, SetLayeredWindowAttributes, SetTimer, SetWindowLongPtrW, ShowWindow,
    TranslateMessage, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, LWA_ALPHA, MSG, SW_SHOWNOACTIVATE,
    WM_DESTROY, WM_PAINT, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

const CLASS_NAME: &str = "MGProfileOverlay";
const WINDOW_WIDTH: i32 = 260;
const WINDOW_HEIGHT: i32 = 64;
/// How far above the screen's bottom edge the popup sits -- clear of the
/// taskbar on a default Windows setup.
const BOTTOM_MARGIN: i32 = 96;
const TIMER_ID: usize = 1;
/// "a few seconds" per explicit instruction, short enough to not linger
/// but long enough to actually read.
const DISPLAY_DURATION_MS: u32 = 1500;
/// Out of 255 -- solid enough to read clearly, translucent enough to still
/// read as a transient overlay rather than a real window.
const WINDOW_ALPHA: u8 = 235;

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);

            let mut rect = RECT::default();
            let _ = GetClientRect(hwnd, &mut rect);
            let bg = CreateSolidBrush(COLORREF(0x00202020)); // near-black, BGR order
            FillRect(hdc, &rect, bg);
            let _ = DeleteObject(bg);

            let text_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Vec<u16>;
            if !text_ptr.is_null() {
                let font = CreateFontW(
                    -22,
                    0,
                    0,
                    0,
                    FW_BOLD.0 as i32,
                    0,
                    0,
                    0,
                    windows::Win32::Graphics::Gdi::DEFAULT_CHARSET.0 as u32,
                    windows::Win32::Graphics::Gdi::OUT_DEFAULT_PRECIS.0 as u32,
                    windows::Win32::Graphics::Gdi::CLIP_DEFAULT_PRECIS.0 as u32,
                    windows::Win32::Graphics::Gdi::CLEARTYPE_QUALITY.0 as u32,
                    windows::Win32::Graphics::Gdi::FF_DONTCARE.0 as u32,
                    PCWSTR::null(),
                );
                let old_font = SelectObject(hdc, font);
                SetTextColor(hdc, COLORREF(0x00FFFFFF));
                SetBkMode(hdc, TRANSPARENT);
                let mut text = (*text_ptr).clone();
                DrawTextW(
                    hdc,
                    &mut text,
                    &mut rect,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                );
                SelectObject(hdc, old_font);
                let _ = DeleteObject(font);
            }

            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_TIMER => {
            let _ = KillTimer(hwnd, TIMER_ID);
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            let text_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Vec<u16>;
            if !text_ptr.is_null() {
                drop(Box::from_raw(text_ptr));
            }
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn register_class_once() {
    static REGISTERED: Once = Once::new();
    REGISTERED.call_once(|| unsafe {
        let instance = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
        let class_name = to_wide(CLASS_NAME);
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        // Leaked deliberately: the class name string must outlive every
        // window created from it, and this class lives for the process's
        // whole lifetime (registered once, never unregistered).
        std::mem::forget(class_name);
        RegisterClassExW(&class);
    });
}

/// Shows a transient "Chat/Game (Profile N)" popup near the bottom of
/// whichever monitor currently holds the foreground window (the game, in
/// the normal case) for `DISPLAY_DURATION_MS`, then closes itself. Returns
/// immediately -- the actual window lives on its own thread.
pub fn show(profile: u8, chat: bool) {
    register_class_once();
    let text = format!(
        "{} (profile {})",
        if chat { "Chat" } else { "Game" },
        profile
    );

    std::thread::spawn(move || unsafe {
        // The player is looking at whatever monitor the foreground
        // window (the game) is on, which is not necessarily the primary
        // monitor `GetSystemMetrics(SM_CXSCREEN/SM_CYSCREEN)` alone would
        // assume -- a popup on the wrong screen in a multi-monitor setup
        // would go unseen. `MONITOR_DEFAULTTOPRIMARY` falls back to the
        // primary monitor only if there is no foreground window at all.
        let monitor = MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let rect = if GetMonitorInfoW(monitor, &mut info).as_bool() {
            info.rcMonitor
        } else {
            RECT {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            }
        };
        let x = rect.left + (rect.right - rect.left - WINDOW_WIDTH) / 2;
        let y = rect.bottom - WINDOW_HEIGHT - BOTTOM_MARGIN;

        let class_name = to_wide(CLASS_NAME);
        let window_name = to_wide("");
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            PCWSTR(class_name.as_ptr()),
            PCWSTR(window_name.as_ptr()),
            WS_POPUP,
            x,
            y,
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
            None,
            None,
            GetModuleHandleW(PCWSTR::null()).unwrap_or_default(),
            None,
        ) else {
            log::warn!("overlay: couldn't create the notification popup");
            return;
        };

        // Stash the text for WM_PAINT to read, and freed again in
        // WM_DESTROY -- the window itself carries no other state.
        let boxed_text = Box::into_raw(Box::new(to_wide(&text)));
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, boxed_text as isize);

        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), WINDOW_ALPHA, LWA_ALPHA);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetTimer(hwnd, TIMER_ID, DISPLAY_DURATION_MS, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
}
