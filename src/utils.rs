// Windows never calls this (see `cli::setup_gui_or_console`'s windows branch,
// which decides GUI-vs-console from whether any args were given instead) --
// only actually reachable, and so only compiled, on macOS/Linux, where a
// console-subsystem binary needs its own way to tell "launched from a
// terminal" from "double-clicked in a file manager".
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn is_double_clicked() -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::io::IsTerminal;
        return !std::io::stdout().is_terminal();
    }

    #[cfg(target_os = "linux")]
    {
        use std::io::IsTerminal;
        let has_display =
            std::env::var("DISPLAY").is_ok() || std::env::var("WAYLAND_DISPLAY").is_ok();
        return has_display && !std::io::stdout().is_terminal();
    }
}

/// Give this GUI-subsystem process somewhere to print when it was started
/// from a command line.
///
/// Only handles the process does not already have are bound to the parent
/// console: a handle that is already valid came from the shell (`> file`,
/// `| more`, a pipe from another process), and overwriting it would send the
/// output to the console instead of where the caller asked for it.
pub fn attach_console() {
    #[cfg(target_os = "windows")]
    {
        use std::ffi::c_void;

        extern "system" {
            fn AttachConsole(dwProcessId: u32) -> i32;
            fn GetStdHandle(nStdHandle: u32) -> *mut c_void;
            fn SetStdHandle(nStdHandle: u32, hHandle: *mut c_void) -> i32;
            fn CreateFileA(
                lpFileName: *const u8,
                dwDesiredAccess: u32,
                dwShareMode: u32,
                lpSecurityAttributes: *mut c_void,
                dwCreationDisposition: u32,
                dwFlagsAndAttributes: u32,
                hTemplateFile: *mut c_void,
            ) -> *mut c_void;
        }

        const STD_INPUT_HANDLE: u32 = 0xFFFF_FFF6; // -10
        const STD_OUTPUT_HANDLE: u32 = 0xFFFF_FFF5; // -11
        const STD_ERROR_HANDLE: u32 = 0xFFFF_FFF4; // -12

        const GENERIC_READ: u32 = 0x80000000;
        const GENERIC_WRITE: u32 = 0x40000000;
        const FILE_SHARE_READ: u32 = 1;
        const FILE_SHARE_WRITE: u32 = 2;
        const OPEN_EXISTING: u32 = 3;
        const INVALID_HANDLE_VALUE: isize = -1;

        fn is_valid(h: *mut c_void) -> bool {
            !h.is_null() && h as isize != INVALID_HANDLE_VALUE
        }

        unsafe {
            // Read the inherited handles first: AttachConsole replaces the
            // standard handles with console ones, which would throw away a
            // redirection the caller set up.
            let inherited_out = GetStdHandle(STD_OUTPUT_HANDLE);
            let inherited_err = GetStdHandle(STD_ERROR_HANDLE);
            let inherited_in = GetStdHandle(STD_INPUT_HANDLE);

            if AttachConsole(0xFFFFFFFF) == 0 {
                return;
            }

            if is_valid(inherited_out) || is_valid(inherited_err) {
                if is_valid(inherited_out) {
                    SetStdHandle(STD_OUTPUT_HANDLE, inherited_out);
                }
                if is_valid(inherited_err) {
                    SetStdHandle(STD_ERROR_HANDLE, inherited_err);
                }
            }

            if !is_valid(inherited_out) || !is_valid(inherited_err) {
                let h_out = CreateFileA(
                    "CONOUT$\0".as_ptr(),
                    GENERIC_WRITE,
                    FILE_SHARE_WRITE,
                    std::ptr::null_mut(),
                    OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
                );
                if h_out as isize != INVALID_HANDLE_VALUE {
                    if !is_valid(inherited_out) {
                        SetStdHandle(STD_OUTPUT_HANDLE, h_out);
                    }
                    if !is_valid(inherited_err) {
                        SetStdHandle(STD_ERROR_HANDLE, h_out);
                    }
                }
            }

            if is_valid(inherited_in) {
                SetStdHandle(STD_INPUT_HANDLE, inherited_in);
            } else {
                let h_in = CreateFileA(
                    "CONIN$\0".as_ptr(),
                    GENERIC_READ,
                    FILE_SHARE_READ,
                    std::ptr::null_mut(),
                    OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
                );
                if h_in as isize != INVALID_HANDLE_VALUE {
                    SetStdHandle(STD_INPUT_HANDLE, h_in);
                }
            }
        }
    }
}

pub fn create_no_window(mut cmd: std::process::Command) -> std::process::Command {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let _ = &mut cmd;
    cmd
}
