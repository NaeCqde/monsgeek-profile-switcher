use std::env;
use std::path::PathBuf;

#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::fs;

pub(crate) fn binary_name(_app_type: &str) -> &'static str {
    "monsgeek-profile-switcher"
}

/// Kill any running instance of this app's daemon binary, other than the
/// current process. `launchctl unload` (macOS) / `systemctl --user stop`
/// (Linux) don't reliably kill a process that Launch Services / a prior
/// launch spawned outside that specific job tracking (e.g. macOS `open -a`
/// launches), so this is a belt-and-suspenders sweep by binary name.
pub(crate) fn kill_other_daemon_processes(app_type: &str) {
    let current_pid = std::process::id();
    let name = binary_name(app_type);

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        if let Ok(output) = std::process::Command::new("pgrep")
            .arg("-x")
            .arg(name)
            .output()
        {
            if let Ok(text) = String::from_utf8(output.stdout) {
                for line in text.lines() {
                    if let Ok(pid) = line.trim().parse::<u32>() {
                        if pid != current_pid {
                            let _ = std::process::Command::new("kill")
                                .args(&["-9", &pid.to_string()])
                                .status();
                        }
                    }
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let _ = crate::utils::create_no_window(std::process::Command::new("taskkill"))
            .args(&[
                "/F",
                "/FI",
                &format!("PID ne {}", current_pid),
                "/IM",
                &format!("{}.exe", name),
            ])
            .status();
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn app_name(_app_type: &str) -> &'static str {
    "MonsGeekProfileSwitcher"
}

/// Returns the directory where the app is installed on this platform.
pub(crate) fn install_dir(app_type: &str) -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let local_app_data = env::var("LOCALAPPDATA").unwrap_or_default();
        PathBuf::from(local_app_data)
            .join("Programs")
            .join("monsgeek-profile-switcher")
            .join(app_type)
    }
    #[cfg(target_os = "macos")]
    {
        let home = env::var("HOME").unwrap_or_default();
        PathBuf::from(home)
            .join("Applications")
            .join(format!("{}.app", app_name(app_type)))
    }
    #[cfg(target_os = "linux")]
    {
        let home = env::var("HOME").unwrap_or_default();
        PathBuf::from(home)
            .join(".local")
            .join("bin")
            .join("monsgeek-profile-switcher")
            .join(app_type)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        PathBuf::from(".").join(app_type)
    }
}

/// Returns the full path to the installed executable.
pub(crate) fn installed_exe_path(app_type: &str) -> PathBuf {
    let dir = install_dir(app_type);
    #[cfg(target_os = "windows")]
    {
        dir.join(format!("{}.exe", binary_name(app_type)))
    }
    #[cfg(target_os = "macos")]
    {
        dir.join("Contents")
            .join("MacOS")
            .join(binary_name(app_type))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        dir.join(binary_name(app_type))
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn plist_label(app_type: &str) -> String {
    format!("quest.nae.monsgeek-profile-switcher.{}", app_type)
}

/// Path to this app's per-user LaunchAgent plist (`~/Library/LaunchAgents/<label>.plist`).
#[cfg(target_os = "macos")]
pub(crate) fn launchagent_plist_path(app_type: &str) -> Result<PathBuf, std::env::VarError> {
    Ok(PathBuf::from(env::var("HOME")?)
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{}.plist", plist_label(app_type))))
}

/// Path to this app's installed `.app` bundle (`~/Applications/<AppName>.app`).
#[cfg(target_os = "macos")]
pub(crate) fn installed_app_bundle_path(app_type: &str) -> Result<PathBuf, std::env::VarError> {
    Ok(PathBuf::from(env::var("HOME")?)
        .join("Applications")
        .join(format!("{}.app", app_name(app_type))))
}

/// Absolute path to the `lsregister` tool used to (de)register bundles with
/// Launch Services.
#[cfg(target_os = "macos")]
pub(crate) const LSREGISTER_PATH: &str = "/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister";

#[cfg(target_os = "linux")]
pub(crate) fn service_name(app_type: &str) -> String {
    format!("monsgeek-profile-switcher-{}.service", app_type)
}

#[cfg(target_os = "windows")]
pub(crate) fn registry_run_key(_app_type: &str) -> &'static str {
    "MonsGeekProfileSwitcherDaemon"
}

pub(crate) fn stop_and_unregister(app_type: &str) -> Result<(), Box<dyn std::error::Error>> {
    kill_other_daemon_processes(app_type);

    #[cfg(target_os = "windows")]
    {
        let _ = crate::utils::create_no_window(std::process::Command::new("reg"))
            .args(&[
                "delete",
                "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run",
                "/v",
                registry_run_key(app_type),
                "/f",
            ])
            .status();
    }

    #[cfg(target_os = "macos")]
    {
        let plist_path = launchagent_plist_path(app_type)?;
        let _ = std::process::Command::new("launchctl")
            .args(&["unload", &plist_path.to_string_lossy()])
            .status();
        if plist_path.exists() {
            let _ = fs::remove_file(&plist_path);
        }

        let app_path = installed_app_bundle_path(app_type)?;
        if app_path.exists() {
            let _ = std::process::Command::new(LSREGISTER_PATH)
                .args(&["-u", &app_path.to_string_lossy()])
                .status();
        }
    }

    #[cfg(target_os = "linux")]
    {
        let service = service_name(app_type);
        let _ = std::process::Command::new("systemctl")
            .args(&["--user", "stop", &service])
            .status();
        let _ = std::process::Command::new("systemctl")
            .args(&["--user", "disable", &service])
            .status();

        let home = env::var("HOME")?;
        let service_path = PathBuf::from(home)
            .join(".config")
            .join("systemd")
            .join("user")
            .join(&service);
        if service_path.exists() {
            let _ = fs::remove_file(&service_path);
        }

        let _ = std::process::Command::new("systemctl")
            .args(&["--user", "daemon-reload"])
            .status();
    }

    Ok(())
}
