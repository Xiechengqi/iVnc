//! Unified durable data root for iVnc.
//!
//! All persistent application data lives under `$IVNC_HOME` (default `$HOME/.ivnc`).
//! Runtime sockets (Wayland / Pulse) stay on the system `XDG_RUNTIME_DIR`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const BUILTIN_CHROME_ID: &str = "builtin-chrome";

/// Resolve the iVnc data home: `$IVNC_HOME` or `$HOME/.ivnc`.
pub fn ivnc_home() -> PathBuf {
    if let Ok(override_home) = env::var("IVNC_HOME") {
        let trimmed = override_home.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .map(|h| PathBuf::from(h).join(".ivnc"))
        .unwrap_or_else(|| PathBuf::from("/root/.ivnc"))
}

pub fn console_json() -> PathBuf {
    ivnc_home().join("console.json")
}

pub fn apps_db() -> PathBuf {
    ivnc_home().join("apps.db")
}

pub fn app_running_state() -> PathBuf {
    ivnc_home().join("app_running_state.json")
}

pub fn capability_calls() -> PathBuf {
    ivnc_home().join("capability_calls.jsonl")
}

pub fn default_pidfile() -> PathBuf {
    ivnc_home().join("ivnc.pid")
}

pub fn desktop_dir() -> PathBuf {
    ivnc_home().join("Desktop")
}

pub fn default_upload_dir_string() -> String {
    desktop_dir().to_string_lossy().into_owned()
}

pub fn applications_dir() -> PathBuf {
    ivnc_home().join("applications")
}

pub fn miao_dir() -> PathBuf {
    ivnc_home().join("miao")
}

pub fn app_dir(app_id: &str) -> PathBuf {
    ivnc_home().join("apps").join(app_id)
}

pub fn app_home(app_id: &str) -> PathBuf {
    app_dir(app_id).join("home")
}

pub fn app_data_dir(app_id: &str) -> PathBuf {
    app_dir(app_id).join("data")
}

pub fn app_log_dir(app_id: &str) -> PathBuf {
    app_dir(app_id)
}

pub fn chrome_profile() -> PathBuf {
    app_dir(BUILTIN_CHROME_ID).join("profile")
}

/// Crash dumps dir pinned under the Chrome profile (avoids $HOME/.config leaks).
pub fn chrome_crash_dumps_dir() -> PathBuf {
    chrome_profile().join("Crash Reports")
}

/// Real process HOME (not `$IVNC_HOME`, not per-app isolation HOME).
pub fn host_home() -> PathBuf {
    env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/root"))
}

/// Chrome product dirs that may receive Crash/Metrics outside `--user-data-dir`.
pub fn chrome_sidecar_config_dirs() -> Vec<PathBuf> {
    vec![
        ivnc_home().join(".config").join("google-chrome"),
        host_home().join(".config").join("google-chrome"),
    ]
}

/// Create the standard layout under `$IVNC_HOME`.
pub fn ensure_layout() -> Result<(), String> {
    let home = ivnc_home();
    for dir in [
        home.clone(),
        desktop_dir(),
        applications_dir(),
        miao_dir(),
        home.join("apps"),
        chrome_profile(),
        app_home(BUILTIN_CHROME_ID),
        app_data_dir(BUILTIN_CHROME_ID),
    ] {
        fs::create_dir_all(&dir)
            .map_err(|e| format!("Failed to create {}: {}", dir.display(), e))?;
    }
    Ok(())
}

/// Ensure per-app directories exist and return the isolated HOME path.
pub fn prepare_app_home(app_id: &str) -> Result<PathBuf, String> {
    let home = app_home(app_id);
    fs::create_dir_all(&home)
        .map_err(|e| format!("Failed to create app home {}: {}", home.display(), e))?;
    fs::create_dir_all(app_data_dir(app_id))
        .map_err(|e| format!("Failed to create app data dir for {}: {}", app_id, e))?;
    Ok(home)
}

/// XDG vars that would pin child processes to the real user dirs.
pub const APP_ENV_REMOVE: &[&str] = &["XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME"];

/// Apply isolation env to a std::process::Command (HOME + clear XDG overrides).
/// Keeps Wayland/runtime vars from the parent environment.
pub fn apply_std_command_isolation(cmd: &mut std::process::Command, app_id: &str) -> Result<(), String> {
    let home = prepare_app_home(app_id)?;
    cmd.env("HOME", &home);
    for key in APP_ENV_REMOVE {
        cmd.env_remove(key);
    }
    Ok(())
}

/// Merge isolation into a CLI env map (does not remove keys the caller set explicitly
/// except when they are the XDG overrides we want cleared — those are omitted).
pub fn merge_cli_isolation_env(
    app_id: &str,
    base: &std::collections::HashMap<String, String>,
) -> Result<std::collections::HashMap<String, String>, String> {
    let home = prepare_app_home(app_id)?;
    let mut env = base.clone();
    for key in APP_ENV_REMOVE {
        env.remove(*key);
    }
    env.insert("HOME".to_string(), home.to_string_lossy().into_owned());
    Ok(env)
}

/// One-shot migration from legacy XDG /tmp locations into `$IVNC_HOME`.
/// Existing new paths are left untouched; legacy sources are moved when possible.
pub fn migrate_legacy() {
    let home = ivnc_home();
    if let Err(err) = ensure_layout() {
        log::warn!("ivnc layout ensure failed: {}", err);
        return;
    }

    let legacy_config = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/root/.config"))
        .join("ivnc");
    let legacy_data = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/root/.local/share"))
        .join("ivnc");
    let legacy_share_apps = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/root/.local/share"))
        .join("applications");
    let legacy_cache = dirs::cache_dir()
        .unwrap_or_else(|| {
            env::var("HOME")
                .map(|h| PathBuf::from(h).join(".cache"))
                .unwrap_or_else(|_| PathBuf::from("/root/.cache"))
        })
        .join("ivnc");

    migrate_file(&legacy_config.join("apps.db"), &apps_db());
    migrate_file(&legacy_config.join("console.json"), &console_json());
    migrate_file(
        &legacy_config.join("app_running_state.json"),
        &app_running_state(),
    );
    migrate_file(
        &legacy_data.join("capability_calls.jsonl"),
        &capability_calls(),
    );
    migrate_dir(&legacy_config.join("miao"), &miao_dir());
    migrate_dir(&legacy_config.join("chrome"), &chrome_profile());

    // Per-app logs under ~/.config/ivnc/apps/<id>/
    if let Ok(entries) = fs::read_dir(legacy_config.join("apps")) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(app_id) = name.to_str() else {
                continue;
            };
            let dest = app_dir(app_id);
            migrate_dir_contents_files(&entry.path(), &dest);
        }
    }

    // /tmp/ivnc-apps/<id>/{desktop,background,cli} → apps/<id>/data/
    let tmp_apps = PathBuf::from("/tmp/ivnc-apps");
    if let Ok(entries) = fs::read_dir(&tmp_apps) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(app_id) = name.to_str() else {
                continue;
            };
            let dest = app_data_dir(app_id);
            let _ = fs::create_dir_all(&dest);
            for kind in ["desktop", "background", "cli"] {
                let src = entry.path().join(kind);
                if src.is_dir() {
                    migrate_dir_contents_into(&src, &dest);
                }
            }
        }
    }

    // iVnc-generated desktop entries
    if let Ok(entries) = fs::read_dir(&legacy_share_apps) {
        let dest_root = applications_dir();
        let _ = fs::create_dir_all(&dest_root);
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            if let Ok(content) = fs::read_to_string(&path) {
                if content.contains("Generated by iVnc") {
                    let dest = dest_root.join(entry.file_name());
                    migrate_file(&path, &dest);
                }
            }
        }
    }

    // Optional cache absorb for chrome
    if legacy_cache.is_dir() {
        let dest = app_home(BUILTIN_CHROME_ID).join(".cache").join("ivnc");
        migrate_dir(&legacy_cache, &dest);
    }

    log::info!("iVnc data home ready at {}", home.display());
}

fn migrate_file(src: &Path, dest: &Path) {
    if !src.exists() || dest.exists() {
        return;
    }
    if let Some(parent) = dest.parent() {
        let _ = fs::create_dir_all(parent);
    }
    match fs::rename(src, dest) {
        Ok(()) => log::info!("Migrated {} → {}", src.display(), dest.display()),
        Err(_) => match copy_recursive(src, dest) {
            Ok(()) => {
                let _ = fs::remove_file(src);
                log::info!("Copied {} → {}", src.display(), dest.display());
            }
            Err(err) => log::warn!(
                "Failed to migrate file {} → {}: {}",
                src.display(),
                dest.display(),
                err
            ),
        },
    }
}

fn migrate_dir(src: &Path, dest: &Path) {
    if !src.exists() {
        return;
    }
    if dest.exists() {
        // If destination exists but looks empty-ish, still try to merge contents.
        if dir_is_effectively_empty(dest) {
            migrate_dir_contents_into(src, dest);
        }
        return;
    }
    if let Some(parent) = dest.parent() {
        let _ = fs::create_dir_all(parent);
    }
    match fs::rename(src, dest) {
        Ok(()) => log::info!("Migrated {} → {}", src.display(), dest.display()),
        Err(_) => match copy_recursive(src, dest) {
            Ok(()) => {
                let _ = fs::remove_dir_all(src);
                log::info!("Copied {} → {}", src.display(), dest.display());
            }
            Err(err) => log::warn!(
                "Failed to migrate dir {} → {}: {}",
                src.display(),
                dest.display(),
                err
            ),
        },
    }
}

fn migrate_dir_contents_files(src: &Path, dest: &Path) {
    let _ = fs::create_dir_all(dest);
    let Ok(entries) = fs::read_dir(src) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            let target = dest.join(entry.file_name());
            migrate_file(&path, &target);
        }
    }
}

fn migrate_dir_contents_into(src: &Path, dest: &Path) {
    let _ = fs::create_dir_all(dest);
    let Ok(entries) = fs::read_dir(src) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let target = dest.join(entry.file_name());
        if path.is_dir() {
            migrate_dir(&path, &target);
        } else {
            migrate_file(&path, &target);
        }
    }
}

fn dir_is_effectively_empty(path: &Path) -> bool {
    fs::read_dir(path)
        .map(|mut it| it.next().is_none())
        .unwrap_or(true)
}

fn copy_recursive(src: &Path, dest: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        fs::create_dir_all(dest)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let target = dest.join(entry.file_name());
            copy_recursive(&entry.path(), &target)?;
        }
        Ok(())
    } else {
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src, dest)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn ivnc_home_respects_override() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = std::env::temp_dir().join(format!("ivnc-paths-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        env::set_var("IVNC_HOME", &tmp);
        assert_eq!(ivnc_home(), tmp);
        env::remove_var("IVNC_HOME");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn migrate_moves_legacy_apps_db() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = std::env::temp_dir().join(format!("ivnc-migrate-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let ivnc = tmp.join("ivnc-home");
        let legacy = tmp.join("legacy-config").join("ivnc");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("apps.db"), b"sqlite").unwrap();

        env::set_var("IVNC_HOME", &ivnc);
        // Point dirs::* by setting XDG_CONFIG_HOME
        env::set_var("XDG_CONFIG_HOME", tmp.join("legacy-config"));
        env::set_var("XDG_DATA_HOME", tmp.join("legacy-data"));
        env::set_var("XDG_CACHE_HOME", tmp.join("legacy-cache"));
        fs::create_dir_all(tmp.join("legacy-data")).unwrap();
        fs::create_dir_all(tmp.join("legacy-cache")).unwrap();

        migrate_legacy();
        assert!(apps_db().exists());
        assert_eq!(fs::read(apps_db()).unwrap(), b"sqlite");

        env::remove_var("IVNC_HOME");
        env::remove_var("XDG_CONFIG_HOME");
        env::remove_var("XDG_DATA_HOME");
        env::remove_var("XDG_CACHE_HOME");
        let _ = fs::remove_dir_all(&tmp);
    }
}
