use super::app::{AppType, ManagedApp};
use log::info;
use std::fs;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

/// Log file path for a managed app.
pub fn log_path(app_id: &str) -> std::path::PathBuf {
    let dir = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/root/.config"))
        .join("ivnc")
        .join("apps")
        .join(app_id);
    let _ = fs::create_dir_all(&dir);
    dir.join("app.log")
}

/// Build the launch command for a desktop app.
pub fn build_command(app: &ManagedApp) -> Result<Command, String> {
    if app.app_type != AppType::DesktopApp {
        return Err("WebApp runs as a background service".to_string());
    }

    let mut cmd = build_desktop_command(app)?;
    configure_process_group(&mut cmd);
    Ok(cmd)
}

fn configure_process_group(cmd: &mut Command) {
    unsafe {
        cmd.pre_exec(|| {
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

fn build_desktop_command(app: &ManagedApp) -> Result<Command, String> {
    let exec_cmd = app
        .exec_command
        .as_ref()
        .ok_or("DesktopApp must have exec_command")?;

    info!("Desktop app '{}' launch info:", app.name);
    info!("  Command: {}", exec_cmd);

    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(exec_cmd);

    if let Some(env_vars) = &app.env_vars {
        for (key, value) in env_vars {
            cmd.env(key, value);
            info!("  Env: {}={}", key, value);
        }
    }

    cmd.env_remove("LD_PRELOAD");
    cmd.env_remove("DBUS_SYSTEM_BUS_ADDRESS");
    cmd.env_remove("DBUS_SESSION_BUS_ADDRESS");

    if let Ok(val) = std::env::var("WAYLAND_DISPLAY") {
        cmd.env("WAYLAND_DISPLAY", &val);
    }
    if let Ok(val) = std::env::var("XDG_RUNTIME_DIR") {
        cmd.env("XDG_RUNTIME_DIR", &val);
    }
    if let Ok(val) = std::env::var("DISPLAY") {
        cmd.env("DISPLAY", &val);
    }

    let log_file = log_path(&app.id);
    info!("  Log file: {}", log_file.display());
    let stdout_file =
        fs::File::create(&log_file).map_err(|e| format!("Failed to create log file: {}", e))?;
    let stderr_file = stdout_file
        .try_clone()
        .map_err(|e| format!("Failed to clone log file: {}", e))?;
    cmd.stdout(Stdio::from(stdout_file));
    cmd.stderr(Stdio::from(stderr_file));

    Ok(cmd)
}
