use std::env;
use std::path::{Path, PathBuf};

pub fn get_config_dir(app_type: &str) -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = env::var("APPDATA") {
            PathBuf::from(appdata)
                .join("monsgeek-profile-switcher")
                .join(app_type)
        } else {
            let user_profile = env::var("USERPROFILE").unwrap_or_default();
            PathBuf::from(user_profile)
                .join("AppData")
                .join("Roaming")
                .join("monsgeek-profile-switcher")
                .join(app_type)
        }
    }
    #[cfg(target_os = "macos")]
    {
        let home = env::var("HOME").unwrap_or_default();
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("monsgeek-profile-switcher")
            .join(app_type)
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(xdg) = env::var("XDG_CONFIG_HOME") {
            PathBuf::from(xdg)
                .join("monsgeek-profile-switcher")
                .join(app_type)
        } else {
            let home = env::var("HOME").unwrap_or_default();
            PathBuf::from(home)
                .join(".config")
                .join("monsgeek-profile-switcher")
                .join(app_type)
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        PathBuf::from(".").join(app_type)
    }
}

pub fn get_config_path(app_type: &str) -> PathBuf {
    get_config_dir(app_type).join(".env")
}

pub fn find_inactive_env_path(app_type: &str) -> PathBuf {
    if let Ok(current_exe) = env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            // Check if there is an app_type-specific inactive.env under parent
            let p_app_type = parent.join(app_type).join("inactive.env");
            if p_app_type.exists() {
                return p_app_type;
            }

            let exe_dir_str = parent.to_string_lossy();
            if exe_dir_str.contains(".app/Contents/MacOS") {
                if let Some(contents) = parent.parent() {
                    if let Some(app_root) = contents.parent() {
                        // Under macOS App Translocation the .app is remounted at a
                        // temporary read-only path.  Use SecTranslocate to resolve the
                        // original location so we can find the inactive.env that sits
                        // next to the real .app in the download folder.
                        #[cfg(target_os = "macos")]
                        let real_app_root = macos_de_translocate(app_root);
                        #[cfg(not(target_os = "macos"))]
                        let real_app_root = app_root.to_path_buf();

                        if let Some(app_parent) = real_app_root.parent() {
                            let p_app_type = app_parent.join(app_type).join("inactive.env");
                            if p_app_type.exists() {
                                return p_app_type;
                            }
                            let p = app_parent.join("inactive.env");
                            if p.exists() {
                                return p;
                            }
                            // Canonical fallback: always a meaningful path even if the
                            // file does not yet exist (avoids "/" when Finder sets cwd).
                            return p_app_type;
                        }
                    }
                }
            }

            // Skip checking parent.join("inactive.env") if we are running from a target directory
            let is_in_target = exe_dir_str.contains("target/release")
                || exe_dir_str.contains("target/debug")
                || exe_dir_str.contains("target/x86_64")
                || exe_dir_str.contains("target/aarch64");
            if !is_in_target {
                let p = parent.join("inactive.env");
                if p.exists() {
                    return p;
                }
            }
        }
    }
    let p_app_type = env::current_dir()
        .unwrap_or_default()
        .join(app_type)
        .join("inactive.env");
    if p_app_type.exists() {
        return p_app_type;
    }
    let p = env::current_dir().unwrap_or_default().join("inactive.env");
    if p.exists() {
        return p;
    }
    p
}

/// Resolve the real filesystem path of a macOS App-Translocated bundle.
/// If the path is not translocated, returns it unchanged.
#[cfg(target_os = "macos")]
fn macos_de_translocate(path: &Path) -> PathBuf {
    use std::ffi::CString;
    const KCF_STRING_ENCODING_UTF8: u32 = 0x08000100;
    const KCF_URL_POSIX_PATH_STYLE: i64 = 0;

    #[link(name = "CoreFoundation", kind = "framework")]
    #[link(name = "Security", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithCString(
            alloc: *const std::ffi::c_void,
            c_str: *const std::ffi::c_char,
            encoding: u32,
        ) -> *mut std::ffi::c_void;
        fn CFURLCreateWithFileSystemPath(
            alloc: *const std::ffi::c_void,
            path: *mut std::ffi::c_void,
            style: i64,
            is_dir: bool,
        ) -> *mut std::ffi::c_void;
        fn CFURLGetFileSystemRepresentation(
            url: *const std::ffi::c_void,
            resolve: bool,
            buf: *mut u8,
            len: i64,
        ) -> bool;
        fn SecTranslocateIsTranslocatedURL(
            url: *mut std::ffi::c_void,
            is_translocated: *mut bool,
            error: *mut *mut std::ffi::c_void,
        ) -> bool;
        fn SecTranslocateCreateOriginalPathForURL(
            url: *mut std::ffi::c_void,
            error: *mut *mut std::ffi::c_void,
        ) -> *mut std::ffi::c_void;
        fn CFRelease(cf: *const std::ffi::c_void);
    }

    let path_str = match path.to_str() {
        Some(s) => s,
        None => return path.to_path_buf(),
    };
    let c_path = match CString::new(path_str) {
        Ok(s) => s,
        Err(_) => return path.to_path_buf(),
    };

    unsafe {
        let cf_str =
            CFStringCreateWithCString(std::ptr::null(), c_path.as_ptr(), KCF_STRING_ENCODING_UTF8);
        if cf_str.is_null() {
            return path.to_path_buf();
        }

        let cf_url =
            CFURLCreateWithFileSystemPath(std::ptr::null(), cf_str, KCF_URL_POSIX_PATH_STYLE, true);
        CFRelease(cf_str);
        if cf_url.is_null() {
            return path.to_path_buf();
        }

        let mut is_translocated = false;
        let ok =
            SecTranslocateIsTranslocatedURL(cf_url, &mut is_translocated, std::ptr::null_mut());
        if !ok || !is_translocated {
            CFRelease(cf_url);
            return path.to_path_buf();
        }

        let original_url = SecTranslocateCreateOriginalPathForURL(cf_url, std::ptr::null_mut());
        CFRelease(cf_url);
        if original_url.is_null() {
            return path.to_path_buf();
        }

        let mut buf = vec![0u8; 4096];
        let ok = CFURLGetFileSystemRepresentation(
            original_url,
            true,
            buf.as_mut_ptr(),
            buf.len() as i64,
        );
        CFRelease(original_url);
        if !ok {
            return path.to_path_buf();
        }

        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        match std::str::from_utf8(&buf[..end]) {
            Ok(s) => PathBuf::from(s),
            Err(_) => path.to_path_buf(),
        }
    }
}

pub fn load_env(app_type: &str) {
    // Load all sources, lowest priority first, so later calls override earlier ones.
    // Priority (highest wins): 3. cwd .env  >  2. OS config .env  >  1. existing env vars
    //                         >  0. inactive.env (only when not installed and no cwd .env).
    //
    // dotenvy::from_path_override overwrites already-set env vars; from_path does not.

    let installed = get_config_path(app_type);
    let cwd_env = env::current_dir().unwrap_or_default().join(".env");

    // Source 0: inactive.env — pre-install preview only.
    // Used when neither the OS config .env nor a cwd .env exist, so the status
    // display shows meaningful defaults before the user runs the installer.
    if !installed.exists() && !cwd_env.exists() {
        let inactive = find_inactive_env_path(app_type);
        if inactive.exists() {
            let _ = dotenvy::from_path(&inactive); // does NOT override existing env vars
        }
    }

    // Source 2: OS-specific installed config folder.
    if installed.exists() {
        let _ = dotenvy::from_path_override(&installed);
    }

    // Source 3: current working directory .env — highest priority.
    if cwd_env.exists() {
        let _ = dotenvy::from_path_override(&cwd_env);
    }
}

pub fn open_dir_in_file_manager(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer").arg(path).spawn()?;
    }
    #[cfg(target_os = "macos")]
    {
        if path.extension().map_or(false, |ext| ext == "app") {
            // open -R on a file inside the bundle triggers Finder's package contents view
            let env_path = path.join(".env");
            let reveal = if env_path.exists() {
                env_path
            } else {
                path.to_path_buf()
            };
            std::process::Command::new("open")
                .arg("-R")
                .arg(&reveal)
                .spawn()?;
        } else {
            std::process::Command::new("open").arg(path).spawn()?;
        }
    }
    #[cfg(target_os = "linux")]
    {
        if std::env::var("DISPLAY").is_ok() || std::env::var("WAYLAND_DISPLAY").is_ok() {
            std::process::Command::new("xdg-open").arg(path).spawn()?;
        }
    }
    Ok(())
}

pub fn show_config(app_type: &str) -> Result<(), Box<dyn std::error::Error>> {
    let config_path = get_config_path(app_type);
    println!("{}", config_path.to_string_lossy());
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
        open_dir_in_file_manager(parent)?;
    }
    Ok(())
}

/// Returns the port that the daemon listens on for the given app_type.
pub fn get_daemon_port(_app_type: &str) -> u16 {
    SwitcherConfig::load().status_port
}

fn parse_profile(var: &str, default: u8) -> u8 {
    env::var(var)
        .ok()
        .and_then(|s| s.trim().parse::<u8>().ok())
        .filter(|n| (1..=4).contains(n))
        .unwrap_or(default)
}

/// On-keyboard profile slots are 1..=4 in the official driver UI.
/// Wire protocol uses 0..=3.
#[derive(Debug, Clone)]
pub struct SwitcherConfig {
    pub on_connect_profile: u8,
    pub profile_chat: u8,
    pub profile_game: u8,
    pub status_port: u16,
    /// The key (optionally with modifiers, e.g. "Ctrl+Alt+Home") that
    /// manually toggles between `profile_chat` and `profile_game` -- the
    /// daemon's real switching signal on Windows (see
    /// `windows_events::run`), replacing automatic detection (IME state,
    /// and before that screen-based caret/OCR detection) entirely: both
    /// proved too unreliable in some games to trust over a key the user
    /// presses on purpose. Parsed by `hotkey::parse_hotkey`.
    pub profile_toggle_hotkey: String,
}

impl SwitcherConfig {
    pub fn load() -> Self {
        let status_port = env::var("STATUS_PORT")
            .ok()
            .and_then(|s| s.trim().parse::<u16>().ok())
            .filter(|p| *p != 0)
            .unwrap_or(38415);
        Self {
            on_connect_profile: parse_profile("ON_CONNECT_PROFILE", 2),
            profile_chat: parse_profile("PROFILE_CHAT", 2),
            profile_game: parse_profile("PROFILE_GAME", 1),
            status_port,
            profile_toggle_hotkey: env::var("PROFILE_TOGGLE_HOTKEY")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "Home".to_string()),
        }
    }

    pub fn default_env_contents() -> &'static str {
        "ON_CONNECT_PROFILE=2\nPROFILE_CHAT=2\nPROFILE_GAME=1\nSTATUS_PORT=38415\nPROFILE_TOGGLE_HOTKEY=Home\n"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_env_contents_match_spec() {
        let s = SwitcherConfig::default_env_contents();
        assert!(s.contains("ON_CONNECT_PROFILE=2"));
        assert!(s.contains("PROFILE_CHAT=2"));
        assert!(s.contains("PROFILE_GAME=1"));
    }
}
