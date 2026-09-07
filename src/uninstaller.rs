use crate::installer_utils::stop_and_unregister;
use std::fs;
use std::path::Path;

#[cfg(any(target_os = "windows", target_os = "linux"))]
use crate::installer_utils::binary_name;
#[cfg(target_os = "macos")]
use crate::installer_utils::installed_app_bundle_path;
#[cfg(any(target_os = "windows", target_os = "linux"))]
use std::env;
#[cfg(any(target_os = "windows", target_os = "linux"))]
use std::path::PathBuf;

fn remove_script_files(target_dir: &Path) {
    #[cfg(target_os = "windows")]
    let files = &["uninstall.bat", "config.bat"];
    #[cfg(target_os = "macos")]
    let files = &["uninstall.command", "config.command"];
    #[cfg(target_os = "linux")]
    let files = &["uninstall.sh", "config.sh"];

    for file_name in files {
        let path = target_dir.join(file_name);
        if path.exists() {
            let _ = fs::remove_file(path);
        }
    }
}

fn remove_dir_if_empty_or_ds_store(dir: &Path) {
    if dir.exists() && dir.is_dir() {
        let mut is_empty_or_ds = true;
        let mut ds_store_path = None;
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                if entry.file_name() == ".DS_Store" {
                    ds_store_path = Some(entry.path());
                } else {
                    is_empty_or_ds = false;
                    break;
                }
            }
        }
        if is_empty_or_ds {
            if let Some(ds_path) = ds_store_path {
                let _ = fs::remove_file(ds_path);
            }
            let _ = fs::remove_dir(dir);
        }
    }
}

pub fn uninstall(app_type: &str) -> Result<(), Box<dyn std::error::Error>> {
    let _ = stop_and_unregister(app_type);

    #[cfg(target_os = "windows")]
    {
        let local_app_data = env::var("LOCALAPPDATA")?;
        let install_dir = PathBuf::from(local_app_data)
            .join("Programs")
            .join("monsgeek-profile-switcher")
            .join(app_type);
        let target_exe = install_dir.join(format!("{}.exe", binary_name(app_type)));

        remove_script_files(&install_dir);

        if target_exe.exists() {
            let _ = fs::remove_file(&target_exe);
        }
        remove_dir_if_empty_or_ds_store(&install_dir);
        if let Some(parent_dir) = install_dir.parent() {
            remove_dir_if_empty_or_ds_store(parent_dir);
        }
    }

    #[cfg(target_os = "macos")]
    {
        let app_path = installed_app_bundle_path(app_type)?;
        remove_script_files(&app_path);
        if app_path.exists() {
            let _ = fs::remove_dir_all(&app_path);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let home = env::var("HOME")?;
        let bin_dir = PathBuf::from(home)
            .join(".local")
            .join("bin")
            .join("monsgeek-profile-switcher")
            .join(app_type);
        let target_exe = bin_dir.join(binary_name(app_type));

        remove_script_files(&bin_dir);

        if target_exe.exists() {
            let _ = fs::remove_file(&target_exe);
        }
        remove_dir_if_empty_or_ds_store(&bin_dir);
        if let Some(parent_dir) = bin_dir.parent() {
            remove_dir_if_empty_or_ds_store(parent_dir);
        }
    }

    // Delete config file and config directory if empty
    let target_env = crate::config::get_config_path(app_type);
    if target_env.exists() {
        let _ = fs::remove_file(&target_env);
    }
    if let Some(config_dir) = target_env.parent() {
        remove_dir_if_empty_or_ds_store(config_dir);
        if let Some(parent_config_dir) = config_dir.parent() {
            remove_dir_if_empty_or_ds_store(parent_config_dir);
        }
    }

    Ok(())
}
