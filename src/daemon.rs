use crate::config::SwitcherConfig;
use crate::hid;
use hidapi::{DeviceInfo, HidApi};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[cfg(not(target_os = "windows"))]
const POLL_INTERVAL: Duration = Duration::from_millis(250);
/// A client that connects but never sends must not hold the status thread.
const REQUEST_READ_TIMEOUT: Duration = Duration::from_millis(200);

/// What the status endpoint publishes, so the GUI and the console status can
/// show the live profile without opening the HID device themselves and racing
/// the poll loop (or, on Windows, the event handlers) for it.
///
/// `chat`/`game` are computed from `profile` against `profile_chat`/
/// `profile_game` (the actual keyboard profile numbers, from
/// `SwitcherConfig`) rather than tracked as their own flag, so they always
/// reflect what the keyboard is really set to -- including a profile set
/// by some other means entirely, e.g. `--set-profile` -- rather than only
/// the daemon's own idea of which one it last chose.
pub(crate) struct Reported {
    keyboard_connected: bool,
    profile: Option<u8>,
    profile_chat: u8,
    profile_game: u8,
}

impl Reported {
    fn new(profile_chat: u8, profile_game: u8) -> Self {
        Self {
            keyboard_connected: false,
            profile: None,
            profile_chat,
            profile_game,
        }
    }

    /// Every key is always present; an unknown one is `null` rather than
    /// missing, so a consumer never has to tell "absent" from "unset".
    fn body(&self) -> String {
        let chat = self.profile.map(|p| p == self.profile_chat);
        let game = self.profile.map(|p| p == self.profile_game);
        format!(
            r#"{{"connected":{},"profile":{},"chat":{},"game":{}}}"#,
            self.keyboard_connected,
            json_or_null(self.profile),
            json_or_null(chat),
            json_or_null(game),
        )
    }
}

fn json_or_null<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map_or_else(|| "null".to_string(), |v| v.to_string())
}

pub(crate) type SharedStatus = Arc<Mutex<Reported>>;

/// A loopback listener that answers "the daemon is up, and here is what it
/// sees". `check_status` and the GUI probe this port instead of scanning
/// processes.
fn spawn_status_server(port: u16, status: SharedStatus) {
    std::thread::spawn(move || {
        let addr = format!("127.0.0.1:{}", port);
        let listener = match TcpListener::bind(&addr) {
            Ok(l) => l,
            Err(e) => {
                log::error!("Failed to bind status port {}: {}", port, e);
                return;
            }
        };
        log::info!("Status listener on http://{}/", addr);
        for stream in listener.incoming().flatten() {
            let body = status.lock().map(|s| s.body()).unwrap_or_default();
            let mut stream = stream;

            // Read the request before answering. Replying and closing while
            // the client is still sending makes Windows reset the connection,
            // which the client sees as a failed read rather than a response.
            let _ = stream.set_read_timeout(Some(REQUEST_READ_TIMEOUT));
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);

            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .as_bytes(),
            );
            let _ = stream.flush();
        }
    });
}

/// Tracks what the keyboard is believed to be set to, so we only touch HID on
/// an actual change, and only log a given failure once instead of every poll.
#[derive(Default)]
pub(crate) struct ProfileState {
    pub(crate) last_sent: Option<u8>,
    failed_for: Option<u8>,
}

impl ProfileState {
    pub(crate) fn forget(&mut self) {
        self.last_sent = None;
        self.failed_for = None;
    }

    pub(crate) fn apply(&mut self, api: &HidApi, info: &DeviceInfo, desired: u8, reason: &str) {
        if self.last_sent == Some(desired) {
            return;
        }
        match hid::set_profile(api, info, desired) {
            Ok(()) => {
                log::info!("Profile {} ({})", desired, reason);
                self.last_sent = Some(desired);
                self.failed_for = None;
            }
            Err(e) => {
                if self.failed_for != Some(desired) {
                    log::warn!(
                        "Failed to set profile {} ({}); is the official driver closed? {}",
                        desired,
                        reason,
                        e
                    );
                    self.failed_for = Some(desired);
                }
            }
        }
    }
}

/// Windows drives the profile switch entirely from events -- a hotkey press
/// (`windows_events::run`) and HID device arrival/removal notifications.
/// macOS has no hotkey or device-watch wiring, so it polls: its only real
/// job is noticing a fresh connection and applying `ON_CONNECT_PROFILE`.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = SwitcherConfig::load();
    let status: SharedStatus = Arc::new(Mutex::new(Reported::new(
        config.profile_chat,
        config.profile_game,
    )));
    spawn_status_server(config.status_port, Arc::clone(&status));

    #[cfg(target_os = "windows")]
    {
        crate::windows_events::run(config, status)
    }

    #[cfg(target_os = "macos")]
    {
        // The HID poll runs off the main thread so the main thread can stay in
        // -[NSApplication run]. That keeps this process registered with Launch
        // Services as the running instance of the bundle, so double-clicking
        // the app opens the status window (see mac_apple_events) instead of
        // hanging on an Apple Event nobody answers.
        std::thread::spawn(move || {
            if let Err(e) = hid_loop(config, status) {
                log::error!("HID loop stopped: {}", e);
            }
        });
        crate::gui::init_nsapplication_accessory();
        crate::mac_apple_events::install_open_application_handler();
        crate::gui::run_nsapplication_forever();
        Ok(())
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        hid_loop(config, status)
    }
}

/// Poll-based fallback for platforms with no event-driven wiring of their
/// own (see `run`'s doc comment) -- just notices a fresh connection and
/// applies `ON_CONNECT_PROFILE`; no chat/game switching happens here at
/// all, matching those platforms never having had automatic detection.
#[cfg(not(target_os = "windows"))]
fn hid_loop(
    config: SwitcherConfig,
    status: SharedStatus,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut api = HidApi::new()?;
    let mut last_path: Option<String> = None;
    let mut state = ProfileState::default();

    loop {
        if let Err(e) = api.refresh_devices() {
            log::warn!("HID refresh failed: {}", e);
        }

        match hid::find_keyboard(&api) {
            None => {
                if last_path.take().is_some() {
                    log::info!("MonsGeek keyboard disconnected");
                    state.forget();
                }
                publish(&status, false, None);
            }
            Some(info) => {
                let path = info.path().to_string_lossy().into_owned();
                if last_path.as_deref() != Some(path.as_str()) {
                    log::info!(
                        "MonsGeek keyboard connected (pid={:04x}), applying ON_CONNECT_PROFILE={}",
                        info.product_id(),
                        config.on_connect_profile
                    );
                    state.forget();
                    // Retry: right after enumeration the interface is often not
                    // openable yet.
                    match hid::set_profile_with_retry(&api, &info, config.on_connect_profile, 8) {
                        Ok(()) => state.last_sent = Some(config.on_connect_profile),
                        Err(e) => log::warn!(
                            "Failed to set ON_CONNECT_PROFILE; is the official driver closed? {}",
                            e
                        ),
                    }
                    last_path = Some(path);
                }

                publish(&status, true, state.last_sent);
            }
        }

        std::thread::sleep(POLL_INTERVAL);
    }
}

pub(crate) fn publish(status: &SharedStatus, connected: bool, profile: Option<u8>) {
    if let Ok(mut s) = status.lock() {
        s.keyboard_connected = connected;
        s.profile = profile;
    }
}
