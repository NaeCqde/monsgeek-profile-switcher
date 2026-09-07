//! Windows' event-driven daemon loop: registers two things on one dedicated
//! thread, with one message loop, and reacts the moment either fires.
//!
//! - The profile-toggle hotkey (`RegisterHotKey`). Its message handler
//!   applies the profile switch directly, the instant the key is pressed.
//! - HID device arrival/removal notifications (`RegisterDeviceNotificationW`,
//!   filtered to the HID device-interface class). Windows tells us the
//!   moment the keyboard shows up or goes away.
//!
//! `RegisterHotKey` posts `WM_HOTKEY` to the *thread* that registered it
//! (no window needed); `RegisterDeviceNotificationW` needs a real window to
//! send `WM_DEVICECHANGE` to. Both land in the same `GetMessageW` loop here
//! because a thread's message queue carries both kinds at once -- one
//! thread, one loop.

use crate::config::SwitcherConfig;
use crate::daemon::{publish, ProfileState, SharedStatus};
use crate::hid;
use crate::hotkey::parse_hotkey;
use crate::overlay;
use hidapi::{DeviceInfo, HidApi};
use windows::core::{GUID, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, MOD_NOREPEAT};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, GetWindowLongPtrW,
    PostQuitMessage, RegisterClassExW, RegisterDeviceNotificationW, SetWindowLongPtrW,
    TranslateMessage, CS_HREDRAW, CS_VREDRAW, DBT_DEVICEARRIVAL, DBT_DEVICEREMOVECOMPLETE,
    DBT_DEVTYP_DEVICEINTERFACE, DEVICE_NOTIFY_WINDOW_HANDLE, DEV_BROADCAST_DEVICEINTERFACE_W,
    GWLP_USERDATA, HWND_MESSAGE, MSG, WM_DESTROY, WM_DEVICECHANGE, WM_HOTKEY, WNDCLASSEXW,
};

const HOTKEY_ID: i32 = 1;
const CLASS_NAME: &str = "MGDaemonEvents";

/// The HID device-interface class GUID (`GUID_DEVINTERFACE_HID`,
/// `4D1E55B2-F16F-11CF-88CB-001111000030`) -- built by hand rather than
/// pulled from a `Win32_Devices_*` feature, since the constant's value is
/// stable ABI (it identifies the class, not a specific device) and this
/// avoids a guess at which crate feature exposes it.
const GUID_DEVINTERFACE_HID: GUID = GUID::from_values(
    0x4D1E55B2,
    0xF16F,
    0x11CF,
    [0x88, 0xCB, 0x00, 0x11, 0x11, 0x00, 0x00, 0x30],
);

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Everything the window procedure and the hotkey branch of the message
/// loop both need, stashed via `GWLP_USERDATA` since a raw `extern
/// "system"` window procedure can't capture a closure. Every access to it
/// happens on this module's own single event-loop thread (the message
/// loop that owns it), so plain field access through a raw pointer is
/// sound without any locking.
struct EventContext {
    config: SwitcherConfig,
    status: SharedStatus,
    api: HidApi,
    state: ProfileState,
    last_path: Option<String>,
    chat: bool,
}

impl EventContext {
    /// Re-enumerates HID devices and reacts if the MonsGeek keyboard's
    /// presence changed since the last call -- shared by the initial
    /// startup check and every `WM_DEVICECHANGE` notification.
    fn recheck_keyboard(&mut self) {
        if let Err(e) = self.api.refresh_devices() {
            log::warn!("HID refresh failed: {}", e);
        }
        match hid::find_keyboard(&self.api) {
            None => {
                if self.last_path.take().is_some() {
                    log::info!("MonsGeek keyboard disconnected");
                    self.state.forget();
                    publish(&self.status, false, None);
                }
            }
            Some(info) => {
                let path = info.path().to_string_lossy().into_owned();
                if self.last_path.as_deref() != Some(path.as_str()) {
                    self.apply_on_connect(&info);
                    self.last_path = Some(path);
                    publish(&self.status, true, self.state.last_sent);
                }
            }
        }
    }

    fn apply_on_connect(&mut self, info: &DeviceInfo) {
        log::info!(
            "MonsGeek keyboard connected (pid={:04x}), applying ON_CONNECT_PROFILE={}",
            info.product_id(),
            self.config.on_connect_profile
        );
        self.state.forget();
        // Retry: right after enumeration the interface is often not openable
        // yet.
        match hid::set_profile_with_retry(&self.api, info, self.config.on_connect_profile, 8) {
            Ok(()) => {
                self.state.last_sent = Some(self.config.on_connect_profile);
                // The keyboard is now on ON_CONNECT_PROFILE, so realign the
                // toggle state with it -- otherwise the first hotkey press
                // after a (re)connect toggles from a stale value and can
                // announce a switch to the state we are already in.
                self.chat = self.config.on_connect_profile == self.config.profile_chat;
            }
            Err(e) => log::warn!(
                "Failed to set ON_CONNECT_PROFILE; is the official driver closed? {}",
                e
            ),
        }
    }

    /// A hotkey press: flip `chat`, and -- only if the keyboard is actually
    /// here right now -- send the matching profile and show the on-screen
    /// notification.
    fn toggle_chat(&mut self) {
        self.chat = !self.chat;
        let profile = if self.chat {
            self.config.profile_chat
        } else {
            self.config.profile_game
        };
        let reason = if self.chat {
            "hotkey: chat"
        } else {
            "hotkey: game"
        };

        match hid::find_keyboard(&self.api) {
            Some(info) => {
                self.state.apply(&self.api, &info, profile, reason);
                publish(&self.status, true, self.state.last_sent);
            }
            None => {
                log::debug!("hotkey pressed with no keyboard connected -- nothing to send");
                publish(&self.status, false, None);
            }
        }
        log::info!(
            "hotkey: profile toggled -- Profile {} ({})",
            profile,
            if self.chat { "chat" } else { "game" }
        );
        overlay::show(profile, self.chat);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_DEVICECHANGE => {
            let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut EventContext;
            if !ctx_ptr.is_null() {
                let event = wparam.0 as u32;
                if event == DBT_DEVICEARRIVAL || event == DBT_DEVICEREMOVECOMPLETE {
                    (*ctx_ptr).recheck_keyboard();
                }
            }
            LRESULT(1)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn register_class(instance: windows::Win32::Foundation::HMODULE) -> windows::core::Result<()> {
    let class_name = to_wide(CLASS_NAME);
    let class = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wndproc),
        hInstance: instance.into(),
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..Default::default()
    };
    // Leaked deliberately: the class name string must outlive the window
    // (registered once for the daemon's whole lifetime).
    std::mem::forget(class_name);
    let atom = unsafe { RegisterClassExW(&class) };
    if atom == 0 {
        return Err(windows::core::Error::from_win32());
    }
    Ok(())
}

/// Runs forever on the calling thread: registers the profile-toggle hotkey
/// and HID device-change notifications, applies `ON_CONNECT_PROFILE`
/// immediately if the keyboard is already connected, then dispatches every
/// hotkey press and device change to `EventContext` as it happens.
pub fn run(config: SwitcherConfig, status: SharedStatus) -> Result<(), Box<dyn std::error::Error>> {
    let hotkey_spec = config.profile_toggle_hotkey.clone();
    let hotkey = parse_hotkey(&hotkey_spec);
    if hotkey.is_none() {
        log::error!(
            "hotkey: couldn't parse PROFILE_TOGGLE_HOTKEY {:?} -- profile switching is disabled \
             until this is fixed (see --config); device connect/disconnect handling still runs",
            hotkey_spec
        );
    }

    let api = HidApi::new()?;
    // Seed the toggle state from the profile the keyboard will be put on at
    // connect, so the first hotkey press flips *away* from it. Starting this
    // at a hardcoded `false` while `ON_CONNECT_PROFILE == PROFILE_CHAT` made
    // the first press claim to switch to chat when chat is where we already
    // were. `apply_on_connect` re-seeds it on every (re)connect; this is the
    // value used until the keyboard is first seen.
    let starts_on_chat = config.on_connect_profile == config.profile_chat;
    let mut ctx = Box::new(EventContext {
        config,
        status,
        api,
        state: ProfileState::default(),
        last_path: None,
        chat: starts_on_chat,
    });

    unsafe {
        let instance = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
        register_class(instance.into())?;

        let class_name = to_wide(CLASS_NAME);
        let window_name = to_wide("");
        let hwnd = CreateWindowExW(
            Default::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(window_name.as_ptr()),
            Default::default(),
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            None,
            instance,
            None,
        )?;

        // The window owns nothing else that could outlive it and needs
        // cleaning up first, so a raw pointer stashed for the process's
        // whole lifetime (this loop never returns) is fine to just leak.
        let ctx_ptr = Box::into_raw(ctx);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, ctx_ptr as isize);
        ctx = Box::from_raw(ctx_ptr);
        std::mem::forget(ctx); // ownership now lives at ctx_ptr until process exit

        let mut filter = DEV_BROADCAST_DEVICEINTERFACE_W {
            dbcc_size: std::mem::size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32,
            dbcc_devicetype: DBT_DEVTYP_DEVICEINTERFACE.0 as u32,
            dbcc_classguid: GUID_DEVINTERFACE_HID,
            ..Default::default()
        };
        let _notification_handle = RegisterDeviceNotificationW(
            hwnd,
            &mut filter as *mut _ as *mut std::ffi::c_void,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        )?;

        // Apply ON_CONNECT_PROFILE right away if the keyboard is already
        // plugged in -- no need to wait for a device-change notification
        // for a device that was already here before we started watching.
        (*ctx_ptr).recheck_keyboard();

        if let Some((modifiers, vk)) = hotkey {
            // MOD_NOREPEAT: without it, holding the key down fires
            // WM_HOTKEY repeatedly at the OS key-repeat rate, which would
            // rapidly flap the toggle for as long as it's held instead of
            // switching once per press.
            if let Err(e) = RegisterHotKey(HWND::default(), HOTKEY_ID, modifiers | MOD_NOREPEAT, vk)
            {
                log::error!(
                    "hotkey: couldn't register {:?}: {} -- is another application already using \
                     it?",
                    hotkey_spec,
                    e
                );
            } else {
                log::info!("hotkey: {:?} toggles the profile", hotkey_spec);
            }
        }

        let mut msg = MSG::default();
        loop {
            let got = GetMessageW(&mut msg, HWND::default(), 0, 0);
            if !got.as_bool() {
                break;
            }
            if msg.message == WM_HOTKEY && msg.wParam.0 as i32 == HOTKEY_ID {
                (*ctx_ptr).toggle_chat();
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let _ = UnregisterHotKey(HWND::default(), HOTKEY_ID);
    }

    Ok(())
}
