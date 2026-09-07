use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:warning=Running workspace post-build script...");
    let build_command = env::var("CRATE_BUILD_COMMAND").unwrap_or_default();

    let is_macos = build_command.contains("apple-darwin")
        || cfg!(target_os = "macos")
        || env::var("CARGO_CFG_TARGET_OS")
            .map(|s| s == "macos")
            .unwrap_or(false);

    if is_macos {
        let out_dir = env::var("CRATE_OUT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("target/release"));
        println!(
            "cargo:warning=macOS target detected. Out dir: {:?}",
            out_dir
        );

        let crate_manifest_path = env::var("CRATE_MANIFEST_PATH").unwrap_or_default();
        let is_switcher = crate_manifest_path.contains("switcher")
            || build_command.contains("monsgeek_profile_switcher");

        let mut targets = Vec::new();
        if is_switcher {
            targets.push((
                "monsgeek-profile-switcher",
                "MonsGeekProfileSwitcher",
                "quest.nae.monsgeek-profile-switcher.switcher",
            ));
        }

        for (bin_name, app_name, bundle_id) in targets {
            let bin_path = out_dir.join(bin_name);
            if bin_path.exists() {
                println!("cargo:warning=Binary found. Packaging {}.app...", app_name);
                let app_dir = out_dir.join(format!("{}.app", app_name));
                if app_dir.exists() {
                    fs::remove_dir_all(&app_dir).expect("Failed to remove existing .app bundle");
                }
                let macos_dir = app_dir.join("Contents").join("MacOS");
                fs::create_dir_all(&macos_dir).expect("Failed to create MacOS directory");

                let bin_dest = macos_dir.join(bin_name);
                fs::copy(&bin_path, &bin_dest).expect("Failed to copy binary");
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mut perms = fs::metadata(&bin_dest)
                        .expect("Failed to get metadata")
                        .permissions();
                    perms.set_mode(0o755);
                    fs::set_permissions(&bin_dest, perms).expect("Failed to set permissions");
                }

                // Create Info.plist
                let info_plist = app_dir.join("Contents").join("Info.plist");
                let plist_content = format!(
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>{}</string>
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
                    bundle_id, app_name, bin_name
                );
                fs::write(info_plist, plist_content).expect("Failed to write Info.plist");
                println!("cargo:warning={} macOS app bundle created successfully.", app_name);

                // Ad-hoc codesign so macOS Gatekeeper accepts the bundle
                let sign_status = std::process::Command::new("codesign")
                    .args(["--sign", "-", "--force", "--deep"])
                    .arg(&app_dir)
                    .status();
                match sign_status {
                    Ok(s) if s.success() => {
                        println!("cargo:warning={}.app ad-hoc signed.", app_name);
                    }
                    Ok(s) => {
                        println!("cargo:warning=codesign exited with status {} for {}.app.", s, app_name);
                    }
                    Err(e) => {
                        println!("cargo:warning=Failed to run codesign for {}.app: {}", app_name, e);
                    }
                }

                // Create a flat, uncompressed zip containing the .app at the root
                let zip_name = format!("{}.app.zip", app_name);
                let zip_path = out_dir.join(&zip_name);
                if zip_path.exists() {
                    fs::remove_file(&zip_path).ok();
                }
                let status = std::process::Command::new("zip")
                    .arg("-r")
                    .arg("-0") // store only, no compression
                    .arg(&zip_path)
                    .arg(format!("{}.app", app_name))
                    .current_dir(&out_dir)
                    .status();
                match status {
                    Ok(s) if s.success() => {
                        println!("cargo:warning={} created (uncompressed).", zip_name);
                    }
                    Ok(s) => {
                        println!("cargo:warning=zip exited with status {} for {}.", s, zip_name);
                    }
                    Err(e) => {
                        println!("cargo:warning=Failed to run zip for {}: {}", zip_name, e);
                    }
                }

                // Stage zip and inactive.env to workspace root for release action.
                // crate_manifest_path is .../Cargo.toml, directly in the repo
                // root now that `shared` and `switcher` are merged into one
                // crate whose manifest lives there.
                let workspace_root = PathBuf::from(&crate_manifest_path)
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| PathBuf::from("."));

                let staged_zip = workspace_root.join(&zip_name);
                if let Err(e) = fs::copy(&zip_path, &staged_zip) {
                    println!("cargo:warning=Failed to stage {}: {}", zip_name, e);
                } else {
                    println!("cargo:warning=Staged {} to workspace root.", zip_name);
                }

            }
        }
    }
}
