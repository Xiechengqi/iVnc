use super::app::ManagedApp;
use std::path::{Path, PathBuf};

const BUILTIN_CHROME_ID: &str = "builtin-chrome";

/// Persistent data root for a managed app (`$IVNC_HOME/apps/<id>/data`).
#[allow(dead_code)]
pub fn data_dir(app: &ManagedApp) -> PathBuf {
    crate::paths::app_data_dir(&app.id)
}

/// App root under `$IVNC_HOME/apps/<id>` (data + home + profile + logs).
pub fn app_root(app: &ManagedApp) -> PathBuf {
    crate::paths::app_dir(&app.id)
}

/// Directories that hold durable app state (cleared by clear-data).
fn durable_dirs(app: &ManagedApp) -> Vec<PathBuf> {
    let mut dirs = vec![
        crate::paths::app_data_dir(&app.id),
        crate::paths::app_home(&app.id),
    ];
    if app.id == BUILTIN_CHROME_ID {
        dirs.push(crate::paths::chrome_profile());
        dirs.extend(crate::paths::chrome_sidecar_config_dirs());
    }
    dirs
}

fn should_recreate_after_clear(dir: &Path) -> bool {
    // Keep the managed app layout ready; leave sidecar ~/.config/google-chrome absent.
    dir.starts_with(crate::paths::ivnc_home().join("apps"))
}

pub fn dir_size(path: &PathBuf) -> u64 {
    if !path.exists() {
        return 0;
    }
    std::fs::read_dir(path)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| {
                    let p = e.path();
                    if p.is_file() {
                        e.metadata().map(|m| m.len()).unwrap_or(0)
                    } else {
                        dir_size(&p)
                    }
                })
                .sum()
        })
        .unwrap_or(0)
}

/// Total durable data size for an app (data + home + chrome profile + sidecars).
pub fn durable_size(app: &ManagedApp) -> u64 {
    let mut total: u64 = durable_dirs(app).iter().map(dir_size).sum();
    if app.id == BUILTIN_CHROME_ID {
        total = total.saturating_add(chrome_tmp_singleton_size());
    }
    total
}

pub fn clear(app: &ManagedApp) -> Result<(), String> {
    for dir in durable_dirs(app) {
        if dir.exists() {
            std::fs::remove_dir_all(&dir)
                .map_err(|e| format!("Failed to clear {}: {}", dir.display(), e))?;
        }
        if should_recreate_after_clear(&dir) {
            std::fs::create_dir_all(&dir)
                .map_err(|e| format!("Failed to recreate {}: {}", dir.display(), e))?;
        }
    }
    if app.id == BUILTIN_CHROME_ID {
        clear_chrome_tmp_singletons();
    }
    Ok(())
}

fn chrome_tmp_singleton_size() -> u64 {
    let Ok(entries) = std::fs::read_dir("/tmp") else {
        return 0;
    };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|n| n.starts_with("com.google.Chrome."))
                .unwrap_or(false)
        })
        .map(|e| dir_size(&e.path()))
        .sum()
}

fn clear_chrome_tmp_singletons() {
    let Ok(entries) = std::fs::read_dir("/tmp") else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with("com.google.Chrome.") {
            continue;
        }
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if let Err(err) = std::fs::remove_dir_all(&path) {
            log::warn!("Failed to remove {}: {}", path.display(), err);
        } else {
            log::info!("Removed Chrome tmp singleton dir {}", path.display());
        }
    }
}

pub fn size_human(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::app::AppType;

    fn chrome_app() -> ManagedApp {
        ManagedApp {
            id: BUILTIN_CHROME_ID.to_string(),
            name: "Chrome".to_string(),
            app_type: AppType::DesktopApp,
            autostart: false,
            url: None,
            launch_command: None,
            launch_env_vars: None,
            launch_cwd: None,
            launch_wait_timeout_secs: None,
            exec_command: None,
            env_vars: None,
            cli_binary_path: None,
            cli_env_vars: None,
            created_at: String::new(),
        }
    }

    #[test]
    fn chrome_durable_dirs_include_sidecars() {
        let dirs = durable_dirs(&chrome_app());
        assert!(dirs.iter().any(|p| p.ends_with("profile")));
        assert!(dirs.iter().any(|p| {
            p.file_name().and_then(|n| n.to_str()) == Some("google-chrome")
        }));
        assert!(dirs.len() >= 5, "expected data+home+profile+2 sidecars, got {}", dirs.len());
    }
}
