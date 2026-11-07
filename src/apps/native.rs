use super::app::{AppType, ManagedApp};
use std::fs;
use std::os::unix::process::CommandExt;
use std::process::Command;

/// Log file path for a managed app.
pub fn log_path(app_id: &str) -> std::path::PathBuf {
    let dir = crate::paths::app_log_dir(app_id);
    let _ = fs::create_dir_all(&dir);
    dir.join("app.log")
}

/// Build the launch command for a desktop app.
pub fn build_command(app: &ManagedApp) -> Result<Command, String> {
    if app.app_type != AppType::DesktopApp {
        return Err("BackgroundApp runs as a background process".to_string());
    }

    let mut cmd = crate::remote::worker::command(app)?;
    configure_process_group(&mut cmd);
    Ok(cmd)
}

fn configure_process_group(cmd: &mut Command) {
    unsafe {
        cmd.pre_exec(|| {
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}
