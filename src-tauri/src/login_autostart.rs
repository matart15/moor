use tauri::AppHandle;

#[cfg(all(target_os = "macos", debug_assertions))]
const LOGIN_AUTOSTART_DEBUG_ERROR: &str = "Auto-start on Login cannot be enabled from a development build. Build and launch the packaged Moor.app, then enable Auto-start on Login there.";

pub(crate) fn validate_login_autostart_enable(_enabled: bool) -> Result<(), String> {
    #[cfg(all(target_os = "macos", debug_assertions))]
    if _enabled {
        return Err(LOGIN_AUTOSTART_DEBUG_ERROR.to_string());
    }

    Ok(())
}

#[cfg(target_os = "macos")]
mod macos {
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    use tauri::{AppHandle, Manager};

    pub(super) fn login_agent_needs_reload(app: &AppHandle) -> bool {
        let app_name = app.package_info().name.clone();
        let Some(path) = launch_agent_path(app, &app_name) else {
            return false;
        };
        launch_agent_file_has_debug_executable(&path)
            || loaded_launch_agent_has_debug_executable(&app_name, &path)
    }

    pub(super) fn refresh_stale_login_agent(app: &AppHandle) {
        let app_name = app.package_info().name.clone();
        let Some(path) = launch_agent_path(app, &app_name) else {
            return;
        };
        if !login_agent_needs_reload(app) {
            return;
        }
        if let Err(error) = reload_login_agent(&app_name, &path) {
            eprintln!(
                "Failed to reload Moor login agent after rewriting stale debug entry: {error}. It should update after the next login."
            );
        }
    }

    // The autostart plugin writes the login agent on each enable/startup.
    // Amend only our own agent file; launchd uses it at the next login.
    // A normal Quit (exit 0) must not cause a relaunch.
    pub(super) fn ensure_crash_recovery(app: &AppHandle) -> Result<(), String> {
        let app_name = app.package_info().name.clone();
        let path = launch_agent_path(app, &app_name).ok_or("Cannot locate login agent")?;
        let existing = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let Some(revised) = with_crash_recovery(&existing) else {
            return Ok(());
        };
        let temporary = path.with_extension("moor-recovery.tmp");
        fs::write(&temporary, revised).map_err(|e| e.to_string())?;
        if let Ok(metadata) = fs::metadata(&path) {
            fs::set_permissions(&temporary, metadata.permissions()).map_err(|e| e.to_string())?;
        }
        fs::rename(&temporary, &path).map_err(|e| e.to_string())
    }

    fn with_crash_recovery(content: &str) -> Option<String> {
        if content.contains("<key>KeepAlive</key>") {
            return None;
        }
        let marker = "<key>RunAtLoad</key>";
        if !content.contains(marker) {
            return None;
        }
        Some(content.replacen(
            marker,
            "<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>\n                <key>ThrottleInterval</key><integer>10</integer>\n                <key>RunAtLoad</key>",
            1,
        ))
    }

    fn is_debug_executable_path(path: &str) -> bool {
        let normalized = path.replace('\\', "/");
        normalized.contains("/src-tauri/target/debug/") || normalized.contains("/target/debug/")
    }

    fn first_launch_agent_program_argument(content: &str) -> Option<String> {
        let key_offset = content.find("<key>ProgramArguments</key>")?;
        let after_key = &content[key_offset..];
        let start_marker = "<string>";
        let end_marker = "</string>";
        let value_start = after_key.find(start_marker)? + start_marker.len();
        let after_start = &after_key[value_start..];
        let value_end = after_start.find(end_marker)?;
        Some(
            after_start[..value_end]
                .replace("&amp;", "&")
                .replace("&quot;", "\"")
                .replace("&apos;", "'")
                .replace("&lt;", "<")
                .replace("&gt;", ">"),
        )
    }

    fn launch_agent_plist_has_debug_executable(content: &str) -> bool {
        first_launch_agent_program_argument(content)
            .as_deref()
            .map(is_debug_executable_path)
            .unwrap_or(false)
    }

    fn launchctl_print_has_debug_executable(content: &str) -> bool {
        content
            .lines()
            .filter_map(|line| line.trim().strip_prefix("program = "))
            .any(is_debug_executable_path)
    }

    fn launch_agent_path(app: &AppHandle, app_name: &str) -> Option<PathBuf> {
        app.path().home_dir().ok().map(|home| {
            home.join("Library")
                .join("LaunchAgents")
                .join(format!("{app_name}.plist"))
        })
    }

    fn user_id_for_launch_agent(path: &Path) -> Option<u32> {
        use std::os::unix::fs::MetadataExt;

        fs::metadata(path).map(|metadata| metadata.uid()).ok()
    }

    fn launch_agent_file_has_debug_executable(path: &Path) -> bool {
        fs::read_to_string(path)
            .map(|content| launch_agent_plist_has_debug_executable(&content))
            .unwrap_or(false)
    }

    fn loaded_launch_agent_has_debug_executable(app_name: &str, path: &Path) -> bool {
        let Some(uid) = user_id_for_launch_agent(path) else {
            return false;
        };
        let service = format!("gui/{uid}/{app_name}");
        let Ok(output) = std::process::Command::new("launchctl")
            .args(["print", &service])
            .output()
        else {
            return false;
        };
        launchctl_print_has_debug_executable(&String::from_utf8_lossy(&output.stdout))
    }

    fn reload_login_agent(app_name: &str, path: &Path) -> Result<(), String> {
        let uid = user_id_for_launch_agent(path)
            .ok_or_else(|| format!("Unable to inspect login agent owner for {}", path.display()))?;
        let domain = format!("gui/{uid}");
        let service = format!("{domain}/{app_name}");

        match std::process::Command::new("launchctl")
            .args(["bootout", &service])
            .status()
        {
            Ok(status) if status.success() => {}
            Ok(status) => eprintln!("Failed to unload existing Moor login agent: {status}"),
            Err(error) => {
                eprintln!("Failed to run launchctl bootout for Moor login agent: {error}")
            }
        }

        let status = std::process::Command::new("launchctl")
            .arg("bootstrap")
            .arg(&domain)
            .arg(path)
            .status()
            .map_err(|err| err.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("launchctl bootstrap failed with status {status}"))
        }
    }

    #[cfg(test)]
    mod crash_recovery_tests {
        use super::with_crash_recovery;

        #[test]
        fn keeps_normal_quit_disabled_and_crashes_restarted() {
            let generated = "<dict><key>RunAtLoad</key><true/></dict>";
            let updated = with_crash_recovery(generated).expect("expected update");
            assert!(updated
                .contains("<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>"));
            assert!(updated.contains("<key>ThrottleInterval</key><integer>10</integer>"));
            assert!(with_crash_recovery(&updated).is_none());
        }

        #[test]
        fn never_modifies_unknown_launch_agent_format() {
            assert!(with_crash_recovery("<plist></plist>").is_none());
        }
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn detects_debug_executable_in_launch_agent_plist() {
            let content = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
  <dict>
    <key>ProgramArguments</key>
    <array>
      <string>/Users/dev/moor/src-tauri/target/debug/moor</string>
    </array>
  </dict>
</plist>"#;

            assert!(super::launch_agent_plist_has_debug_executable(content));
        }

        #[test]
        fn ignores_packaged_executable_in_launch_agent_plist() {
            let content = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
  <dict>
    <key>ProgramArguments</key>
    <array>
      <string>/Applications/Moor.app/Contents/MacOS/moor</string>
    </array>
  </dict>
</plist>"#;

            assert!(!super::launch_agent_plist_has_debug_executable(content));
        }

        #[test]
        fn detects_loaded_debug_login_agent_from_launchctl_output() {
            let output = r#"gui/501/Moor = {
  active count = 0
  program = /Users/dev/moor/src-tauri/target/debug/moor
}"#;

            assert!(super::launchctl_print_has_debug_executable(output));
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn login_agent_needs_reload(app: &AppHandle) -> bool {
    macos::login_agent_needs_reload(app)
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn login_agent_needs_reload(_app: &AppHandle) -> bool {
    false
}

#[cfg(target_os = "macos")]
pub(crate) fn refresh_stale_login_agent(app: &AppHandle) {
    macos::refresh_stale_login_agent(app);
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn refresh_stale_login_agent(_app: &AppHandle) {}

#[cfg(target_os = "macos")]
pub(crate) fn ensure_crash_recovery(app: &AppHandle) -> Result<(), String> {
    macos::ensure_crash_recovery(app)
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn ensure_crash_recovery(_app: &AppHandle) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_login_autostart_enable;

    #[test]
    #[cfg(all(target_os = "macos", debug_assertions))]
    fn rejects_enabling_login_autostart_from_debug_build() {
        let error = validate_login_autostart_enable(true).expect_err("debug build should reject");

        assert!(error.contains("development build"));
    }

    #[test]
    fn allows_disabling_login_autostart_from_debug_build() {
        validate_login_autostart_enable(false).expect("disabling autostart should be allowed");
    }
}
