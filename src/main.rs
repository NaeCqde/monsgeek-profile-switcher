#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod cli;
mod config;
mod daemon;
mod gui;
mod hid;
mod installer;
mod installer_utils;
mod status;
mod uninstaller;
mod utils;

#[cfg(target_os = "windows")]
mod hotkey;
#[cfg(target_os = "macos")]
mod mac_apple_events;
#[cfg(target_os = "windows")]
mod overlay;
#[cfg(target_os = "windows")]
mod windows_events;

use hidapi::HidApi;
use std::env;
use std::error::Error;
use std::process::exit;

const APP_TYPE: &str = "switcher";

/// A missing keyboard and one held open by the official driver look the same
/// from here, so the hint covers both.
const NOT_FOUND: &str = "MonsGeek keyboard not found. Plug it in, close the official \
                         MonsGeek driver, then check --list-hid.";

fn print_usage() {
    println!(
        "MonsGeek Profile Switcher\n\n\
        Usage: monsgeek-profile-switcher [command]\n\n\
        (no command)      Show status (opens the status window when double-clicked)\n\
        --daemon          Run the resident switcher in the foreground. Windows: the profile\n\
                          switch is driven by a hotkey (PROFILE_TOGGLE_HOTKEY in .env, see\n\
                          --config; default Home) -- pressing it shows a brief on-screen\n\
                          notification and toggles between PROFILE_CHAT and PROFILE_GAME\n\
        --install         Install, register at login, and start the daemon\n\
        --uninstall       Stop the daemon and remove the installation\n\
        --start, --stop   Start or stop the installed daemon\n\
        --config          Open the folder holding the .env configuration\n\
        --list-hid        List the MonsGeek HID interfaces this machine sees\n\
        --get-profile     Read the keyboard's active profile (1-4)\n\
        --set-profile N   Switch the keyboard to profile N (1-4)"
    );
}

/// Runs one part of what the daemon does on its own, at the same rate, so a
/// side effect on the machine (input stutter, a key that stops repeating) can
/// be pinned to one cause rather than guessed at.
///
/// `hid` sends nothing to the keyboard. `send` re-sends the profile the
/// keyboard is already on, changing no setting. `switch` alternates between
/// two profiles once a second, the way rapid cursor flapping used to make
/// the daemon do back when it read IME/cursor state; it restores the
/// original profile when it finishes.
fn stress(what: &str, seconds: u64) -> Result<(), Box<dyn Error>> {
    let interval = std::time::Duration::from_millis(250);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
    let mut api = HidApi::new()?;
    let mut ticks = 0u64;

    let opens = what == "open";
    let writes = matches!(what, "send" | "switch");
    let device = if writes || opens {
        let info = hid::find_keyboard(&api).ok_or(NOT_FOUND)?;
        let original = hid::get_profile(&api, &info)?.unwrap_or(1);
        println!(
            "Keyboard is on profile {}; will restore it at the end.",
            original
        );
        Some((info, original))
    } else {
        None
    };

    println!(
        "Stressing \"{}\" every 250 ms for {} s. Hold a key down and watch for repeat.",
        what, seconds
    );

    while std::time::Instant::now() < deadline {
        if what == "hid" {
            api.refresh_devices()?;
            let _ = hid::find_keyboard(&api);
        }
        if opens {
            if let Some((info, _)) = &device {
                // Open and drop the handle without sending anything.
                let _ = hid::open_config_device(&api, info);
            }
        }
        if writes {
            if let Some((info, original)) = &device {
                let profile = if what == "switch" {
                    // One flip per second, alternating with the other of 1/2.
                    let other = if *original == 1 { 2 } else { 1 };
                    if (ticks / 4) % 2 == 0 {
                        *original
                    } else {
                        other
                    }
                } else {
                    *original
                };
                if let Err(e) = hid::set_profile(&api, info, profile) {
                    println!("send failed at tick {}: {}", ticks, e);
                }
            }
        }
        ticks += 1;
        std::thread::sleep(interval);
    }

    if writes {
        if let Some((info, original)) = &device {
            hid::set_profile_with_retry(&api, info, *original, 8)?;
            println!("Restored profile {}.", original);
        }
    }

    println!("Done: {} ticks of \"{}\".", ticks, what);
    Ok(())
}

fn print_status() {
    let (is_installed, is_running, exe_path, config_path) =
        crate::installer::check_status(APP_TYPE);
    let config = crate::config::SwitcherConfig::load();

    let exe_display = if is_installed {
        exe_path.to_string_lossy().to_string()
    } else {
        "(Will be set upon installation)".to_string()
    };

    let status_msg = format!(
        "MonsGeek Profile Switcher Status\n\n\
        [Status]\n\
        - Installed:      {}\n\
        - Running:        {}\n\
        - Current:        {}\n\
        - Executable:     {}\n\
        - Configuration:  {}\n\
        - On connect:     profile {}\n\
        - Chat:           profile {}\n\
        - Game:           profile {}\n\
        - Status port:    127.0.0.1:{}",
        if is_installed { "Yes" } else { "No" },
        if is_running { "Yes" } else { "No" },
        crate::status::describe(crate::status::query(config.status_port).as_ref()),
        exe_display,
        config_path.to_string_lossy(),
        config.on_connect_profile,
        config.profile_chat,
        config.profile_game,
        config.status_port,
    );

    println!("{}", status_msg);
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();

    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));
    crate::config::load_env(APP_TYPE);

    if args.len() >= 2 {
        match args[1].as_str() {
            "--help" | "-h" => {
                crate::cli::setup_gui_or_console(APP_TYPE, &args);
                print_usage();
                return Ok(());
            }
            "--stress" => {
                crate::cli::setup_gui_or_console(APP_TYPE, &args);
                let what = args.get(2).map(String::as_str).unwrap_or("");
                let seconds = args
                    .get(3)
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(30);
                if !matches!(what, "hid" | "open" | "send" | "switch") {
                    return Err(
                        "usage: monsgeek-profile-switcher --stress <hid|open|send|switch> [seconds]"
                            .into(),
                    );
                }
                stress(what, seconds)?;
                return Ok(());
            }
            "--list-hid" => {
                crate::cli::setup_gui_or_console(APP_TYPE, &args);
                let api = HidApi::new()?;
                for line in hid::list_monsgeek_devices(&api) {
                    println!("{}", line);
                }
                if hid::find_keyboard(&api).is_none() {
                    println!("No MonsGeek vendor-config interface found (VID 3151, usage FFFF:02 or interface 2).");
                }
                return Ok(());
            }
            "--get-profile" => {
                crate::cli::setup_gui_or_console(APP_TYPE, &args);
                let api = HidApi::new()?;
                let info = hid::find_keyboard(&api).ok_or(NOT_FOUND)?;
                match hid::get_profile(&api, &info)? {
                    Some(n) => println!("Active profile: {}", n),
                    None => println!(
                        "The keyboard answered, but not in the shape we expect. GET_PROFILE \
                         is unverified on this model -- see docs/PROTOCOL.md."
                    ),
                }
                return Ok(());
            }
            "--set-profile" => {
                crate::cli::setup_gui_or_console(APP_TYPE, &args);
                let n = args
                    .get(2)
                    .and_then(|s| s.trim().parse::<u8>().ok())
                    .filter(|n| (1..=4).contains(n))
                    .ok_or("usage: monsgeek-profile-switcher --set-profile <1-4>")?;
                let api = HidApi::new()?;
                let info = hid::find_keyboard(&api).ok_or(NOT_FOUND)?;
                hid::set_profile_with_retry(&api, &info, n, 8)?;
                println!("Set profile {}", n);
                return Ok(());
            }
            _ => {}
        }
    }

    crate::cli::setup_gui_or_console(APP_TYPE, &args);

    if args.len() < 2 {
        print_status();
        exit(0);
    }

    let cmd = &args[1];

    if crate::cli::handle_common_command(cmd, APP_TYPE) {
        return Ok(());
    }

    if cmd == "--daemon" {
        log::info!("Starting monsgeek-profile-switcher daemon...");
        // A user-assigned hotkey is the real switching signal on Windows --
        // it replaces IME-based `windows_chat::is_chat()` (and, before
        // that, screen-based caret/OCR detection) entirely, per explicit
        // instruction: automatic detection proved too unreliable in some
        // games to trust over a key the user presses on purpose. Both it
        // and HID device-change handling are fully event-driven now (see
        // `windows_events.rs`) -- `daemon::run` takes no arguments because
        // there is no separate listener thread's state left to hand it.
        daemon::run()?;
    } else {
        print_status();
        exit(0);
    }

    Ok(())
}
