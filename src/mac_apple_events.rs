/// macOS: handle the "open application" Apple Event Finder sends when the
/// user double-clicks the .app bundle.
///
/// When the persistent `--daemon` process is already running, Launch
/// Services treats it as the checked-in instance of the bundle and routes a
/// plain double-click to *it* (as `kAEOpenApplication` / `kAEReopenApplication`)
/// instead of spawning a fresh process. The daemon runs as an accessory app
/// (`NSApplicationActivationPolicyAccessory`) with no window or menu bar, so
/// it can't visibly respond, and Finder/Dock eventually reports "Application
/// Not Responding" while waiting for an acknowledgement that never produces
/// a UI.
///
/// The fix: when the daemon receives this event, spawn a brand-new instance
/// of the current executable with no arguments. That fresh process is not
/// the checked-in bundle instance, so it takes the normal
/// `is_double_clicked()` -> GUI branch and shows the status window.
use std::ffi::c_void;

const K_CORE_EVENT_CLASS: u32 = 0x6165_7674; // 'aevt'
const K_AE_OPEN_APPLICATION: u32 = 0x6f61_7070; // 'oapp'
const K_AE_REOPEN_APPLICATION: u32 = 0x7261_7070; // 'rapp'

#[repr(C)]
struct AEDesc {
    descriptor_type: u32,
    data_handle: *mut c_void,
}

type HandlerUPP = unsafe extern "C" fn(*const AEDesc, *mut AEDesc, isize) -> i32;

#[link(name = "CoreServices", kind = "framework")]
extern "C" {
    fn AEInstallEventHandler(ec: u32, ei: u32, h: HandlerUPP, r: isize, sys: bool) -> i32;
}

unsafe extern "C" fn on_open_or_reopen_application(
    _event: *const AEDesc,
    _reply: *mut AEDesc,
    _refcon: isize,
) -> i32 {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).spawn();
    }
    0
}

/// Install handlers for `kAEOpenApplication`/`kAEReopenApplication`. Call on
/// the main thread before driving the run loop (e.g. before
/// `crate::gui::run_nsapplication_forever()`).
pub fn install_open_application_handler() {
    unsafe {
        AEInstallEventHandler(
            K_CORE_EVENT_CLASS,
            K_AE_OPEN_APPLICATION,
            on_open_or_reopen_application,
            0,
            false,
        );
        AEInstallEventHandler(
            K_CORE_EVENT_CLASS,
            K_AE_REOPEN_APPLICATION,
            on_open_or_reopen_application,
            0,
            false,
        );
    }
}
