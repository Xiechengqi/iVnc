use super::app::{AppMode, PakeApp};
use std::fs;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;

pub fn data_dir(app: &PakeApp) -> PathBuf {
    // Use /tmp for Chromium data dir because snap-packaged Chromium
    // has confinement that prevents writing to /root/.config
    let base = PathBuf::from("/tmp").join("ivnc-pake-apps").join(&app.id);

    base.join(match app.mode {
        Some(AppMode::Native) => "chrome",
        Some(AppMode::Webview) => "webview",
        None => "desktop",
    })
}

/// Ensure the data directory exists with write permissions for all users
pub fn ensure_data_dir(app: &PakeApp) -> Result<PathBuf, String> {
    let dir = data_dir(app);
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    // Use 0o755: rwxr-xr-x (owner rwx, others rx) - Chrome needs write for SingletonLock
    builder.mode(0o755);
    builder
        .create(&dir)
        .map_err(|e| format!("Failed to create data dir: {}", e))?;

    // Also ensure parent directories are writable
    if let Some(parent) = dir.parent() {
        let mut pbuilder = fs::DirBuilder::new();
        pbuilder.recursive(true);
        pbuilder.mode(0o755);
        let _ = pbuilder.create(parent);
    }

    Ok(dir)
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

pub fn clear(app: &PakeApp) -> Result<(), String> {
    let dir = data_dir(app);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("Failed to clear data: {}", e))?;
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to recreate dir: {}", e))?;
    Ok(())
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
