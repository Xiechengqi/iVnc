use super::app::{PakeApp, AppMode};
use super::native;
use std::path::PathBuf;

fn autostart_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/root/.config"))
        .join("autostart")
}

pub fn set(app: &PakeApp) -> Result<(), String> {
    let dir = autostart_dir();
    let _ = std::fs::create_dir_all(&dir);

    let exec = match app.mode {
        AppMode::Native => {
            let chrome = native::find_chrome().unwrap_or_else(|| "google-chrome".to_string());
            let data = super::datadir::data_dir(app);
            format!("{} --app={} --user-data-dir={}", chrome, app.url, data.display())
        }
        AppMode::Webview => {
            // For webview mode, use curl to call the API to start the app
            format!("curl -X POST http://localhost:8000/api/apps/{}/start", app.id)
        }
    };

    let content = format!(
        "[Desktop Entry]\nType=Application\nName={}\nExec={}\nStartupNotify=true\nX-GNOME-Autostart-enabled=true\n",
        app.name, exec
    );

    std::fs::write(dir.join(format!("pake-{}.desktop", app.id)), content)
        .map_err(|e| format!("Failed to write autostart: {}", e))
}

pub fn remove(app_id: &str) -> Result<(), String> {
    let path = autostart_dir().join(format!("pake-{}.desktop", app_id));
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|e| format!("Failed to remove autostart: {}", e))?;
    }
    Ok(())
}
