use std::process::exit;

pub fn setup_gui_or_console(app_type: &'static str, args: &[String]) {
    #[cfg(target_os = "windows")]
    {
        if args.len() < 2 {
            crate::gui::run_gui(app_type);
            exit(0);
        } else {
            crate::utils::attach_console();
        }
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        if args.len() < 2 && crate::utils::is_double_clicked() {
            crate::gui::run_gui(app_type);
            exit(0);
        }
    }
}

/// Handles --config, --install, --uninstall, --start, --stop.
/// Returns true if the command was handled (caller should return Ok(())).
/// Calls exit(1) on error.
pub fn handle_common_command(cmd: &str, app_type: &str) -> bool {
    let label = "MonsGeek Profile Switcher";
    match cmd {
        "--config" => {
            let _ = crate::config::show_config(app_type);
            exit(0);
        }
        "--install" => {
            log::info!("Installing monsgeek-profile-switcher...");
            match crate::installer::install(app_type) {
                Ok(_) => {
                    println!("{} installation completed successfully!", label);
                    let _ = crate::config::show_config(app_type);
                }
                Err(e) => {
                    println!("Installation failed: {}", e);
                    exit(1);
                }
            }
        }
        "--uninstall" => {
            log::info!("Uninstalling monsgeek-profile-switcher...");
            match crate::installer::uninstall(app_type) {
                Ok(_) => println!("{} uninstallation completed successfully!", label),
                Err(e) => {
                    println!("Uninstallation failed: {}", e);
                    exit(1);
                }
            }
        }
        "--start" => {
            log::info!("Starting monsgeek-profile-switcher daemon...");
            match crate::installer::start_daemon(app_type) {
                Ok(_) => println!("{} daemon started successfully!", label),
                Err(e) => {
                    println!("Failed to start {} daemon: {}", app_type, e);
                    exit(1);
                }
            }
        }
        "--stop" => {
            log::info!("Stopping monsgeek-profile-switcher daemon...");
            match crate::installer::stop_daemon(app_type) {
                Ok(_) => println!("{} daemon stopped successfully!", label),
                Err(e) => {
                    println!("Failed to stop {} daemon: {}", app_type, e);
                    exit(1);
                }
            }
        }
        _ => return false,
    }
    true
}
