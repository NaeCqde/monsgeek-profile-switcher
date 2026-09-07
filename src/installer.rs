use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::installer_utils::{
    install_dir, installed_exe_path, kill_other_daemon_processes, stop_and_unregister,
};
pub use crate::uninstaller::uninstall;

#[cfg(target_os = "windows")]
use crate::installer_utils::registry_run_key;
#[cfg(target_os = "linux")]
use crate::installer_utils::service_name;
#[cfg(target_os = "macos")]
use crate::installer_utils::{
    app_name, binary_name, launchagent_plist_path, plist_label, LSREGISTER_PATH,
};

fn copy_script_files(target_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "linux")]
    {
        let files = &["uninstall.sh", "config.sh"];

        let current_exe_dir = env::current_exe()?
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        let current_dir = env::current_dir()?;

        for file_name in files {
            let mut src = current_exe_dir.join(*file_name);
            if !src.exists() {
                src = current_dir.join(*file_name);
            }
            if !src.exists() {
                if file_name.starts_with("uninstall") {
                    src = current_dir
                        .join("scripts")
                        .join("uninstall")
                        .join(file_name);
                } else if file_name.starts_with("config") {
                    src = current_dir.join("scripts").join("config").join(file_name);
                }
            }

            if src.exists() {
                let dest = target_dir.join(file_name);
                log::info!("Copying script {:?} to {:?}", src, dest);
                fs::copy(&src, &dest)?;
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&dest)?.permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&dest, perms)?;
            } else {
                log::warn!(
                    "Source script file {:?} not found, skipping copy",
                    file_name
                );
            }
        }
    }

    let _ = target_dir;
    Ok(())
}

fn copy_env_file(app_type: &str) -> Result<(), Box<dyn std::error::Error>> {
    let target_env = crate::config::get_config_path(app_type);
    let config_dir = target_env.parent().ok_or("Invalid config dir")?;
    fs::create_dir_all(config_dir)?;

    let source_env = crate::config::find_inactive_env_path(app_type);

    if !target_env.exists() {
        if source_env.exists() {
            log::info!("Copying inactive.env configuration to {:?}", target_env);
            fs::copy(&source_env, &target_env)?;
        } else {
            log::info!("Writing a default .env file");
            fs::write(
                &target_env,
                crate::config::SwitcherConfig::default_env_contents(),
            )?;
        }
    }
    Ok(())
}

pub fn install(app_type: &str) -> Result<(), Box<dyn std::error::Error>> {
    let _ = stop_and_unregister(app_type);
    let current_exe = env::current_exe()?;

    #[cfg(target_os = "windows")]
    {
        let inst_dir = install_dir(app_type);
        fs::create_dir_all(&inst_dir)?;
        let target_exe = installed_exe_path(app_type);

        copy_env_file(app_type)?;

        if current_exe != target_exe {
            fs::copy(&current_exe, &target_exe)?;
        }

        setup_windows_startup(app_type, &target_exe)?;

        let _ = copy_script_files(&inst_dir);

        crate::utils::create_no_window(std::process::Command::new(&target_exe))
            .arg("--daemon")
            .spawn()?;
    }

    #[cfg(target_os = "macos")]
    {
        let home = env::var("HOME")?;

        let apps_dir = PathBuf::from(home).join("Applications");
        fs::create_dir_all(&apps_dir)?;
        let target_exe = setup_macos_app_bundle(app_type, &apps_dir, &current_exe)?;

        copy_env_file(app_type)?;

        let app_dir = install_dir(app_type);
        register_macos_app(&app_dir);
        setup_macos_launchagent(app_type, &target_exe)?;

        copy_script_files(&app_dir)?;
    }

    #[cfg(target_os = "linux")]
    {
        let bin_dir = install_dir(app_type);
        fs::create_dir_all(&bin_dir)?;

        copy_env_file(app_type)?;

        let target_exe = installed_exe_path(app_type);
        if current_exe != target_exe {
            fs::copy(&current_exe, &target_exe)?;
        }
        setup_linux_systemd(app_type, &target_exe)?;

        let _ = copy_script_files(&bin_dir);
    }

    Ok(())
}

pub fn check_status(app_type: &str) -> (bool, bool, PathBuf, PathBuf) {
    let exe_path = installed_exe_path(app_type);

    #[cfg(target_os = "windows")]
    let is_installed = {
        let status = crate::utils::create_no_window(std::process::Command::new("reg"))
            .args(&[
                "query",
                "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run",
                "/v",
                registry_run_key(app_type),
            ])
            .output();
        status.map(|o| o.status.success()).unwrap_or(false)
    };

    #[cfg(target_os = "macos")]
    let is_installed = launchagent_plist_path(app_type)
        .map(|p| p.exists())
        .unwrap_or(false);

    #[cfg(target_os = "linux")]
    let is_installed = {
        let home = env::var("HOME").unwrap_or_default();
        let service_path = PathBuf::from(home)
            .join(".config")
            .join("systemd")
            .join("user")
            .join(service_name(app_type));
        service_path.exists()
    };

    crate::config::load_env(app_type);

    let port = crate::config::get_daemon_port(app_type);

    let is_running = std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        std::time::Duration::from_millis(200),
    )
    .is_ok();

    let config_path = if is_installed {
        crate::config::get_config_path(app_type)
    } else {
        crate::config::find_inactive_env_path(app_type)
    };

    (is_installed, is_running, exe_path, config_path)
}

#[cfg(target_os = "windows")]
fn setup_windows_startup(
    app_type: &str,
    binary_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let binary_str = format!("\"{}\" --daemon", binary_path.to_string_lossy());
    let status = crate::utils::create_no_window(std::process::Command::new("reg"))
        .args(&[
            "add",
            "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run",
            "/v",
            registry_run_key(app_type),
            "/t",
            "REG_SZ",
            "/d",
            &binary_str,
            "/f",
        ])
        .status()?;
    if !status.success() {
        return Err("Failed to register startup in Windows registry".into());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn get_parent_app_dir(exe_path: &Path) -> Option<PathBuf> {
    let parent = exe_path.parent()?; // MacOS
    if parent.file_name()?.to_str()? == "MacOS" {
        let contents = parent.parent()?; // Contents
        if contents.file_name()?.to_str()? == "Contents" {
            let app = contents.parent()?; // <AppName>.app
            if app.extension()?.to_str()? == "app" {
                return Some(app.to_path_buf());
            }
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> std::io::Result<()> {
    fs::create_dir_all(&dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()))?;
        } else {
            fs::copy(entry.path(), dst.as_ref().join(entry.file_name()))?;
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn setup_macos_app_bundle(
    app_type: &str,
    install_dir: &Path,
    current_exe: &Path,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let name = app_name(app_type);
    let app_dir = install_dir.join(format!("{}.app", name));
    let bin_name = binary_name(app_type);
    let target_exe = app_dir.join("Contents").join("MacOS").join(bin_name);

    if let Some(src_app_dir) = get_parent_app_dir(current_exe) {
        log::info!("Installing from .app bundle: {:?}", src_app_dir);
        if app_dir.exists() {
            log::info!("Removing existing .app bundle at {:?}", app_dir);
            fs::remove_dir_all(&app_dir)?;
        }
        log::info!("Copying .app bundle to {:?}", app_dir);
        copy_dir_all(&src_app_dir, &app_dir)?;
    } else {
        // Fallback: build app bundle dynamically
        log::info!("Installing raw binary. Rebuilding .app bundle structure...");
        let macos_dir = app_dir.join("Contents").join("MacOS");
        fs::create_dir_all(&macos_dir)?;

        if current_exe != target_exe {
            fs::copy(current_exe, &target_exe)?;
        }

        // Write a minimal Info.plist for a background (LSUIElement) agent.
        let info_plist = app_dir.join("Contents").join("Info.plist");
        let plist_content = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>quest.nae.monsgeek-profile-switcher.{}</string>
    <key>CFBundleName</key>
    <string>{}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleSignature</key>
    <string>????</string>
    <key>CFBundleExecutable</key>
    <string>{}</string>
    <key>LSUIElement</key>
    <true/>
    <key>LSMultipleInstancesProhibited</key>
    <false/>
</dict>
</plist>
"#,
            app_type, name, bin_name
        );
        fs::write(info_plist, plist_content)?;
    }

    // Ensure target executable has executable permissions
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if target_exe.exists() {
            let mut perms = fs::metadata(&target_exe)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&target_exe, perms)?;
        }
    }

    Ok(target_exe)
}

#[cfg(target_os = "macos")]
fn register_macos_app(app_path: &Path) {
    let _ = std::process::Command::new(LSREGISTER_PATH)
        .arg("-f")
        .arg(app_path)
        .status();
}

#[cfg(target_os = "macos")]
fn setup_macos_launchagent(
    app_type: &str,
    binary_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let home = env::var("HOME")?;
    let plist_dir = PathBuf::from(home).join("Library").join("LaunchAgents");
    fs::create_dir_all(&plist_dir)?;
    let label = plist_label(app_type);
    let plist_path = plist_dir.join(format!("{}.plist", label));

    // binary_path is .../Name.app/Contents/MacOS/<bin>; three parents up is the
    // .app bundle root.
    let app_bundle = binary_path
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .unwrap_or(binary_path);
    let work_dir = app_bundle;

    // Launch via `open -a` (not a raw exec of the inner binary) so that
    // LaunchServicesd registers the resulting process as the running instance
    // of this bundle. Without that, double-clicking the app while the daemon
    // runs spawns a second, competing daemon instead of being routed to the
    // running one (see mac_apple_events).
    let plist_content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{}</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/bin/open</string>
        <string>-a</string>
        <string>{}</string>
        <string>--args</string>
        <string>--daemon</string>
    </array>
    <key>WorkingDirectory</key>
    <string>{}</string>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
</dict>
</plist>
"#,
        label,
        app_bundle.to_string_lossy(),
        work_dir.to_string_lossy()
    );

    fs::write(&plist_path, plist_content)?;

    let _ = std::process::Command::new("launchctl")
        .arg("unload")
        .arg(&plist_path)
        .status();
    let _ = std::process::Command::new("launchctl")
        .arg("load")
        .arg(&plist_path)
        .status();
    let _ = std::process::Command::new("launchctl")
        .arg("start")
        .arg(&label)
        .status();

    Ok(())
}

#[cfg(target_os = "linux")]
fn setup_linux_systemd(
    app_type: &str,
    binary_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let home = env::var("HOME")?;
    let service_dir = PathBuf::from(home)
        .join(".config")
        .join("systemd")
        .join("user");
    fs::create_dir_all(&service_dir)?;
    let service = service_name(app_type);
    let service_path = service_dir.join(&service);

    let service_content = format!(
        r#"[Unit]
Description=MonsGeek Profile Switcher {} Daemon
After=network.target

[Service]
ExecStart={} --daemon
WorkingDirectory={}
Restart=always

[Install]
WantedBy=default.target
"#,
        app_type,
        binary_path.to_string_lossy(),
        binary_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_string_lossy()
    );

    fs::write(&service_path, service_content)?;

    let _ = std::process::Command::new("systemctl")
        .arg("--user")
        .arg("daemon-reload")
        .status();
    let _ = std::process::Command::new("systemctl")
        .arg("--user")
        .arg("enable")
        .arg(&service)
        .status();
    let _ = std::process::Command::new("systemctl")
        .arg("--user")
        .arg("restart")
        .arg(&service)
        .status();

    Ok(())
}

pub fn paths_are_equal(p1: &Path, p2: &Path) -> bool {
    if let (Ok(c1), Ok(c2)) = (p1.canonicalize(), p2.canonicalize()) {
        c1 == c2
    } else {
        let s1 = p1.to_string_lossy().replace('\\', "/");
        let s2 = p2.to_string_lossy().replace('\\', "/");
        #[cfg(target_os = "windows")]
        {
            s1.eq_ignore_ascii_case(&s2)
        }
        #[cfg(not(target_os = "windows"))]
        {
            s1 == s2
        }
    }
}

fn find_exe_to_start(app_type: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let local_path = installed_exe_path(app_type);
    if local_path.exists() {
        Ok(local_path)
    } else {
        let current = env::current_exe()?;
        Ok(current)
    }
}

pub fn start_daemon(app_type: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (is_installed, _, _, _) = check_status(app_type);

    #[cfg(target_os = "macos")]
    {
        if is_installed {
            let plist_path = launchagent_plist_path(app_type)?;
            let _ = std::process::Command::new("launchctl")
                .args(&["load", &plist_path.to_string_lossy()])
                .status();
            let _ = std::process::Command::new("launchctl")
                .args(&["start", &plist_label(app_type)])
                .status();
            return Ok(());
        }
    }

    #[cfg(target_os = "linux")]
    {
        if is_installed {
            let service = service_name(app_type);
            let status = std::process::Command::new("systemctl")
                .args(&["--user", "start", &service])
                .status()?;
            if status.success() {
                return Ok(());
            } else {
                return Err("Failed to start systemd service".into());
            }
        }
    }

    // Default fallback (Windows, or if not installed on mac/linux)
    #[cfg(target_os = "windows")]
    let _ = is_installed;
    let exe = find_exe_to_start(app_type)?;
    crate::utils::create_no_window(std::process::Command::new(&exe))
        .arg("--daemon")
        .spawn()?;

    Ok(())
}

pub fn stop_daemon(app_type: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (is_installed, _, _, _) = check_status(app_type);

    #[cfg(target_os = "macos")]
    if is_installed {
        let plist_path = launchagent_plist_path(app_type)?;
        let _ = std::process::Command::new("launchctl")
            .args(&["unload", &plist_path.to_string_lossy()])
            .status();
    }

    #[cfg(target_os = "linux")]
    if is_installed {
        let _ = std::process::Command::new("systemctl")
            .args(&["--user", "stop", &service_name(app_type)])
            .status();
    }

    #[cfg(target_os = "windows")]
    let _ = is_installed;

    // Sweep any leftover daemon process the service manager didn't stop
    // (e.g. a macOS instance launched by Launch Services via `open -a`).
    kill_other_daemon_processes(app_type);

    Ok(())
}
