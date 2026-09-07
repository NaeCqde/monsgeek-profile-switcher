use eframe::egui;
use std::path::PathBuf;

/// Render `text` as a label that copies itself to the clipboard when clicked.
fn copyable_label(ui: &mut egui::Ui, text: impl Into<String>) {
    let text = text.into();
    let response = ui
        .add(egui::Label::new(&text).sense(egui::Sense::click()))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Click to copy");
    if response.clicked() {
        ui.ctx().copy_text(text);
    }
}

/// Instantiate `NSApplication` with `Accessory` activation policy (no Dock icon).
///
/// Launch Services only treats a process as a fully "checked-in" running
/// instance of its bundle -- eligible to receive Apple Events routed to an
/// already-running app, such as the `kAEReopenApplication` sent when the user
/// double-clicks the .app while the daemon is running -- once `NSApplication`
/// has actually completed `-finishLaunching` / entered `-run`. A bare
/// Carbon-style `AEInstallEventHandler` + `CFRunLoopRun()`, or even just
/// instantiating `NSApp` without running it, is not enough: Launch Services
/// can locate the process but the Apple Event send to it times out
/// (`errAETimeout`, -1712) because the check-in never completes.
#[cfg(target_os = "macos")]
pub fn init_nsapplication_accessory() {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    use objc2_foundation::MainThreadMarker;
    if let Some(mtm) = MainThreadMarker::new() {
        let app = NSApplication::sharedApplication(mtm);
        let _ = app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    }
}

/// Block the main thread in `-[NSApplication run]` forever (returns only on
/// process exit). Unlike a bare `CFRunLoopRun()`, this drives `NSApp` through
/// its normal launch sequence (`-finishLaunching`, etc.), which is what
/// completes the Launch Services check-in needed to receive Apple Events
/// (e.g. `kAEReopenApplication`) sent to an already-running instance of the
/// bundle.
/// Any `AEInstallEventHandler` handlers must be installed before calling this.
#[cfg(target_os = "macos")]
pub fn run_nsapplication_forever() {
    use objc2_app_kit::NSApplication;
    use objc2_foundation::MainThreadMarker;
    if let Some(mtm) = MainThreadMarker::new() {
        let app = NSApplication::sharedApplication(mtm);
        app.run();
    }
}

pub fn run_gui(app_type: &'static str) {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
        use objc2_foundation::MainThreadMarker;
        if let Some(mtm) = MainThreadMarker::new() {
            let app = NSApplication::sharedApplication(mtm);
            let _ = app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        }
    }

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_maximize_button(false),
        ..Default::default()
    };
    let app_name = "MonsGeek Profile Switcher";
    let _ = eframe::run_native(
        app_name,
        native_options,
        Box::new(move |cc| {
            // Customize look and feel
            let mut style = (*cc.egui_ctx.style()).clone();

            // Set modern dark theme colors
            let mut visuals = egui::Visuals::dark();
            visuals.window_rounding = 12.0.into();
            visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(30, 30, 46); // dark slate/navy
            visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(45, 45, 68);
            visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(60, 60, 90);
            visuals.widgets.active.bg_fill = egui::Color32::from_rgb(75, 75, 110);
            visuals.widgets.inactive.rounding = 8.0.into();
            visuals.widgets.hovered.rounding = 8.0.into();
            visuals.widgets.active.rounding = 8.0.into();

            style.visuals = visuals;
            cc.egui_ctx.set_style(style);

            // Spawn background thread to request repaint every 1 second
            let ctx_clone = cc.egui_ctx.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                ctx_clone.request_repaint();
            });

            Ok(Box::new(StatusApp::new(app_type)))
        }),
    );
}

struct StatusApp {
    app_type: &'static str,
    is_installed: bool,
    is_running: bool,
    exe_path: PathBuf,
    config_path: PathBuf,
    message: Option<String>,
    is_error: bool,
    is_running_from_install_dir: bool,
    last_tick: std::time::Instant,
    active_env_content: String,
    env_changed: bool,
    daemon_status: Option<crate::status::DaemonStatus>,
}

impl StatusApp {
    fn new(app_type: &'static str) -> Self {
        let (is_installed, is_running, exe_path, config_path) =
            crate::installer::check_status(app_type);
        let is_running_from_install_dir = if let Ok(current_exe) = std::env::current_exe() {
            crate::installer::paths_are_equal(&current_exe, &exe_path)
        } else {
            false
        };
        let active_env_content = std::fs::read_to_string(&config_path).unwrap_or_default();
        Self {
            app_type,
            is_installed,
            is_running,
            exe_path,
            config_path,
            message: None,
            is_error: false,
            is_running_from_install_dir,
            last_tick: std::time::Instant::now(),
            active_env_content,
            env_changed: false,
            daemon_status: crate::status::query(crate::config::SwitcherConfig::load().status_port),
        }
    }

    fn refresh(&mut self) {
        let (is_installed, is_running, exe_path, config_path) =
            crate::installer::check_status(self.app_type);
        self.is_installed = is_installed;
        self.is_running = is_running;
        self.exe_path = exe_path;
        self.config_path = config_path;
        self.is_running_from_install_dir = if let Ok(current_exe) = std::env::current_exe() {
            crate::installer::paths_are_equal(&current_exe, &self.exe_path)
        } else {
            false
        };
        self.daemon_status =
            crate::status::query(crate::config::SwitcherConfig::load().status_port);
    }
}

impl eframe::App for StatusApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Request repaint in 1s to keep the status updating
        ctx.request_repaint_after(std::time::Duration::from_secs(1));

        if self.last_tick.elapsed() >= std::time::Duration::from_secs(1) {
            self.last_tick = std::time::Instant::now();
            self.refresh();

            if self.is_running {
                let current_env = std::fs::read_to_string(&self.config_path).unwrap_or_default();
                self.env_changed = current_env != self.active_env_content;
            } else {
                self.env_changed = false;
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
          // Scrollable: a fixed window size can't be guessed to fit every
          // monitor's DPI scale at once (confirmed live -- the same window
          // that fit exactly at 1440p/125% cut off the button rows at
          // 1080p/100%), so content that doesn't fit is reachable by
          // scrolling instead of silently clipped with no way to see it.
          egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(15.0);

            ui.vertical_centered(|ui| {
                let title = "MonsGeek Profile Switcher";
                ui.heading(title);
            });

            ui.add_space(15.0);
            ui.separator();
            ui.add_space(15.0);

            // Status display grid
            egui::Grid::new("status_grid")
                .num_columns(2)
                .spacing([20.0, 14.0])
                .show(ui, |ui| {
                    ui.strong("Service Status:");
                    ui.horizontal(|ui| {
                        // Installation status
                        ui.horizontal(|ui| {
                            let dot_color = if self.is_installed {
                                egui::Color32::from_rgb(46, 204, 113) // Green
                            } else {
                                egui::Color32::from_rgb(149, 165, 166) // Gray
                            };
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                            ui.painter().circle_filled(rect.center(), 5.0, dot_color);
                            ui.add_space(4.0);
                            ui.label(if self.is_installed { "Installed" } else { "Not Installed" });
                        });

                        ui.add_space(15.0);
                        ui.label("|");
                        ui.add_space(15.0);

                        // Running status
                        ui.horizontal(|ui| {
                            let dot_color = if self.is_running {
                                egui::Color32::from_rgb(46, 204, 113) // Green
                            } else {
                                egui::Color32::from_rgb(231, 76, 60) // Red
                            };
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                            ui.painter().circle_filled(rect.center(), 5.0, dot_color);
                            ui.add_space(4.0);
                            ui.label(if self.is_running { "Running (Daemon)" } else { "Stopped" });
                        });
                    });
                    ui.end_row();

                    // The fields the daemon publishes, shown as it
                    // publishes them.
                    ui.strong("Keyboard:");
                    match &self.daemon_status {
                        None => ui.weak("(daemon not running)"),
                        Some(s) if s.keyboard_connected => ui.label("Connected"),
                        Some(_) => ui.label("Not connected"),
                    };
                    ui.end_row();

                    ui.strong("Profile:");
                    match self.daemon_status.as_ref().and_then(|s| s.profile) {
                        Some(profile) => ui.label(profile.to_string()),
                        None => ui.weak("-"),
                    };
                    ui.end_row();

                    ui.strong("Chat:");
                    match self.daemon_status.as_ref().and_then(|s| s.chat) {
                        Some(true) => ui.label("Yes"),
                        Some(false) => ui.label("No"),
                        None => ui.weak("-"),
                    };
                    ui.end_row();

                    ui.strong("Game:");
                    match self.daemon_status.as_ref().and_then(|s| s.game) {
                        Some(true) => ui.label("Yes"),
                        Some(false) => ui.label("No"),
                        None => ui.weak("-"),
                    };
                    ui.end_row();

                    ui.strong("Executable Path:");
                    if self.is_installed {
                        copyable_label(ui, self.exe_path.to_string_lossy().to_string());
                    } else {
                        ui.weak("(Will be set upon installation)");
                    }
                    ui.end_row();

                    ui.strong("Configuration:");
                    copyable_label(ui, self.config_path.to_string_lossy().to_string());
                    ui.end_row();

                    {
                        let config = crate::config::SwitcherConfig::load();

                        ui.strong("On connect:");
                        ui.label(format!("profile {}", config.on_connect_profile));
                        ui.end_row();

                        ui.strong("Chat:");
                        ui.label(format!("profile {}", config.profile_chat));
                        ui.end_row();

                        ui.strong("Game:");
                        ui.label(format!("profile {}", config.profile_game));
                        ui.end_row();

                        ui.strong("Status port:");
                        ui.label(config.status_port.to_string());
                        ui.end_row();
                    }
                });

            ui.add_space(15.0);
            ui.separator();
            ui.add_space(15.0);

            let buttons_enabled = !(self.is_running_from_install_dir && !self.is_installed);

            // Action Buttons
            ui.horizontal(|ui| {
                ui.add_space(10.0);

                let install_text = if self.is_installed {
                    "Reinstall / Update"
                } else {
                    "Install Service"
                };

                // Install/Update button
                let install_btn = ui.add_enabled(
                    buttons_enabled,
                    egui::Button::new(install_text).min_size(egui::vec2(140.0, 32.0))
                );
                if install_btn.clicked() {
                    self.message = Some("Installing service...".to_string());
                    self.is_error = false;
                    ctx.request_repaint();
                    match crate::installer::install(self.app_type) {
                        Ok(_) => {
                            self.message = Some("Service successfully installed and started!".to_string());
                            self.is_error = false;
                            // Wait for daemon to start (up to 3 seconds)
                            let port = crate::config::SwitcherConfig::load().status_port;
                            for _ in 0..6 {
                                std::thread::sleep(std::time::Duration::from_millis(500));
                                if std::net::TcpStream::connect_timeout(
                                    &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
                                    std::time::Duration::from_millis(200),
                                ).is_ok() {
                                    break;
                                }
                            }
                            self.active_env_content = std::fs::read_to_string(&self.config_path).unwrap_or_default();
                            self.env_changed = false;
                        }
                        Err(e) => {
                            self.message = Some(format!("Installation failed: {}", e));
                            self.is_error = true;
                        }
                    }
                    self.refresh();
                }

                ui.add_space(10.0);

                // Uninstall button
                let uninstall_btn = ui.add_enabled(
                    buttons_enabled && self.is_installed,
                    egui::Button::new("Uninstall Service").min_size(egui::vec2(140.0, 32.0))
                );
                if uninstall_btn.clicked() {
                    self.message = Some("Uninstalling service...".to_string());
                    self.is_error = false;
                    ctx.request_repaint();
                    match crate::installer::uninstall(self.app_type) {
                        Ok(_) => {
                            self.message = Some("Service successfully uninstalled!".to_string());
                            self.is_error = false;
                            self.active_env_content = String::new();
                            self.env_changed = false;
                        }
                        Err(e) => {
                            self.message = Some(format!("Uninstallation failed: {}", e));
                            self.is_error = true;
                        }
                    }
                    self.refresh();
                }

                ui.add_space(10.0);

                // Open settings folder button
                let folder_btn = ui.add_enabled(
                    buttons_enabled,
                    egui::Button::new("Open Settings Folder").min_size(egui::vec2(160.0, 32.0))
                );
                if folder_btn.clicked() {
                    let dir_to_open = self.config_path.parent().unwrap_or(&self.config_path);
                    if self.is_installed {
                        let _ = std::fs::create_dir_all(dir_to_open);
                    }
                    if let Err(e) = crate::config::open_dir_in_file_manager(dir_to_open) {
                        self.message = Some(format!("Could not open folder: {}", e));
                        self.is_error = true;
                    }
                }
            });

            // Conditionally render Start/Stop Daemon buttons only when app is installed
            if self.is_installed {
                ui.add_space(10.0);

                // Daemon Control Buttons Row
                ui.horizontal(|ui| {
                    ui.add_space(10.0);

                    let start_btn = ui.add_enabled(
                        buttons_enabled && !self.is_running,
                        egui::Button::new("Start Daemon").min_size(egui::vec2(140.0, 32.0))
                    );
                    if start_btn.clicked() {
                        self.message = Some("Starting daemon...".to_string());
                        self.is_error = false;
                        ctx.request_repaint();
                        match crate::installer::start_daemon(self.app_type) {
                            Ok(_) => {
                                self.message = Some("Daemon successfully started!".to_string());
                                self.is_error = false;
                                // Wait for daemon to start (up to 3 seconds)
                                let port = crate::config::SwitcherConfig::load().status_port;
                                for _ in 0..6 {
                                    std::thread::sleep(std::time::Duration::from_millis(500));
                                    if std::net::TcpStream::connect_timeout(
                                        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
                                        std::time::Duration::from_millis(200),
                                    ).is_ok() {
                                        break;
                                    }
                                }
                                self.active_env_content = std::fs::read_to_string(&self.config_path).unwrap_or_default();
                                self.env_changed = false;
                            }
                            Err(e) => {
                                self.message = Some(format!("Failed to start daemon: {}", e));
                                self.is_error = true;
                            }
                        }
                        self.refresh();
                    }

                    ui.add_space(10.0);

                    let stop_btn = ui.add_enabled(
                        buttons_enabled && self.is_running,
                        egui::Button::new("Stop Daemon").min_size(egui::vec2(140.0, 32.0))
                    );
                    if stop_btn.clicked() {
                        self.message = Some("Stopping daemon...".to_string());
                        self.is_error = false;
                        ctx.request_repaint();
                        match crate::installer::stop_daemon(self.app_type) {
                            Ok(_) => {
                                self.message = Some("Daemon successfully stopped!".to_string());
                                self.is_error = false;
                                std::thread::sleep(std::time::Duration::from_millis(500));
                                self.active_env_content = std::fs::read_to_string(&self.config_path).unwrap_or_default();
                                self.env_changed = false;
                            }
                            Err(e) => {
                                self.message = Some(format!("Failed to stop daemon: {}", e));
                                self.is_error = true;
                            }
                        }
                        self.refresh();
                    }
                });

                if self.env_changed {
                    ui.add_space(8.0);
                    ui.vertical_centered(|ui| {
                        ui.colored_label(
                            egui::Color32::from_rgb(230, 126, 34), // Orange
                            "⚠️ Configuration has changed. Please restart the daemon to apply changes."
                        );
                    });
                }
            }

            // Status message feedback
            if let Some(ref msg) = self.message {
                ui.add_space(15.0);
                ui.vertical_centered(|ui| {
                    let text_color = if self.is_error {
                        egui::Color32::from_rgb(231, 76, 60) // Red
                    } else {
                        egui::Color32::from_rgb(46, 204, 113) // Green
                    };
                    ui.colored_label(text_color, msg);
                });
            }
          });
        });
    }
}
