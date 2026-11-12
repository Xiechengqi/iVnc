//! Checks for the session an application joins, and bounded diagnostics for
//! one launch attempt. Argument adaptation lives in `compat`.
use super::error::ApiError;
use serde::Serialize;
use std::ffi::{CStr, CString, OsString};
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

#[derive(Serialize)]
pub struct Identity {
    pub uid: u32,
    pub gid: u32,
    pub user: String,
}

pub fn identity() -> Identity {
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };
    let mut user = uid.to_string();
    let mut buffer = vec![0u8; 16384];
    unsafe {
        let mut entry: libc::passwd = std::mem::zeroed();
        let mut result = std::ptr::null_mut();
        if libc::getpwuid_r(
            uid,
            &mut entry,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        ) == 0
            && !result.is_null()
            && !entry.pw_name.is_null()
        {
            user = CStr::from_ptr(entry.pw_name).to_string_lossy().into_owned();
        }
    }
    Identity { uid, gid, user }
}

fn path_error(code: &'static str, message: &str, path: &Path) -> ApiError {
    ApiError::new(code, format!("{message}: {}", path.display()))
        .param("path", path.to_string_lossy().into_owned())
}

pub(super) fn accessible(path: &Path, mode: i32) -> bool {
    CString::new(path.as_os_str().as_bytes()).is_ok_and(|p| unsafe {
        libc::faccessat(libc::AT_FDCWD, p.as_ptr(), mode, libc::AT_EACCESS) == 0
    })
}

pub fn validate_runtime(path: &Path) -> Result<(), ApiError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| {
        path_error(
            "desktop_runtime_invalid",
            "Cannot access the Wayland runtime directory",
            path,
        )
    })?;
    if !path.is_absolute()
        || !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o700
        || !accessible(path, libc::R_OK | libc::W_OK | libc::X_OK)
    {
        return Err(path_error(
            "desktop_runtime_invalid",
            "Wayland runtime must be an absolute directory owned by the running user with mode 0700",
            path,
        ));
    }
    Ok(())
}

pub fn validate_home(home: &Path) -> Result<(), ApiError> {
    if !home.is_dir() || !accessible(home, libc::R_OK | libc::W_OK | libc::X_OK) {
        return Err(path_error(
            "desktop_home_unavailable",
            "Application HOME must be a readable, writable and searchable directory",
            home,
        ));
    }
    Ok(())
}

pub fn validate_session(home: &Path, session: &[(OsString, OsString)]) -> Result<(), ApiError> {
    validate_home(home)?;
    let get = |key: &str| session.iter().find(|(k, _)| k == key).map(|(_, v)| v);
    let runtime = get("XDG_RUNTIME_DIR")
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            ApiError::new(
                "desktop_runtime_missing",
                "Missing XDG_RUNTIME_DIR; start iVnc in the application's user session",
            )
        })?;
    validate_runtime(Path::new(runtime))?;
    let display = get("WAYLAND_DISPLAY")
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            ApiError::new(
                "desktop_display_missing",
                "Missing WAYLAND_DISPLAY; the application's display is not ready",
            )
        })?;
    let socket = Path::new(runtime).join(display);
    if !fs::metadata(&socket).is_ok_and(|m| m.file_type().is_socket())
        || !accessible(&socket, libc::W_OK)
    {
        return Err(path_error(
            "desktop_display_unavailable",
            "Wayland socket is missing or inaccessible",
            &socket,
        ));
    }
    Ok(())
}

/// Marks the start of this generation, after log rotation and before spawning.
pub struct LogCursor {
    path: PathBuf,
    offset: u64,
    identity: Option<(u64, u64)>,
}

impl LogCursor {
    pub fn new(path: PathBuf) -> Self {
        let metadata = fs::symlink_metadata(&path).ok();
        Self {
            path,
            offset: metadata.as_ref().map_or(0, |m| m.len()),
            identity: metadata.map(|m| (m.dev(), m.ino())),
        }
    }

    pub fn failure(&self) -> Option<ApiError> {
        let log = self.read()?;
        // A corrected attempt has its own diagnostics, even though both runs
        // are supervised by the same reaper and append to the same log.
        let log = log
            .rsplit_once(super::compat::RETRY_MARKER)
            .map_or(log.as_str(), |(_, tail)| tail);
        diagnose(log)
    }

    pub fn read(&self) -> Option<String> {
        // The app can change its log path. Never follow a symlink or block on a FIFO.
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&self.path)
            .ok()?;
        let metadata = file.metadata().ok()?;
        if !metadata.is_file() {
            return None;
        }
        let offset = if self.identity == Some((metadata.dev(), metadata.ino()))
            && metadata.len() >= self.offset
        {
            self.offset
        } else {
            0
        };
        let start = offset.max(metadata.len().saturating_sub(64 * 1024));
        file.seek(SeekFrom::Start(start)).ok()?;
        let mut bytes = Vec::new();
        file.take(64 * 1024).read_to_end(&mut bytes).ok()?;
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }
}

fn diagnose(log: &str) -> Option<ApiError> {
    // These are fatal startup signatures, not all ERROR messages: Electron
    // frequently prints harmless D-Bus / GPU warnings even when fully working.
    let (code, message) = if log.contains("Running as root without --no-sandbox is not supported") {
        ("desktop_root_sandbox", "Electron refuses to run as root with its sandbox. Run iVnc and the app as the same non-root user, or explicitly add --no-sandbox for this app.")
    } else if log.contains("Failed to initialize Wayland platform") {
        ("desktop_wayland_connect", "The app cannot connect to iVnc's Wayland session. Avoid su/sudo login wrappers: they change the user or remove the display environment. Check the runtime directory and socket permissions.")
    } else if log.contains("Missing X server or $DISPLAY") {
        ("desktop_x11_backend", "This app selected X11. Use its native Wayland backend (for Electron: --ozone-platform=wayland).")
    } else if log.contains("No usable sandbox!")
        || log.contains("The SUID sandbox helper binary was found, but is not configured correctly")
    {
        ("desktop_sandbox_unavailable", "The app's sandbox is unavailable. Check its installation and the host's user namespace / AppArmor policy; do not disable sandboxing globally.")
    } else {
        return None;
    };
    Some(ApiError::new(code, message))
}

/// A corrected configuration or explicit manual restart is needed. Unknown
/// crashes and transient display failures retain the regular retry policy.
pub fn permanent(error: &ApiError) -> bool {
    matches!(
        error.code,
        "desktop_root_sandbox"
            | "desktop_x11_backend"
            | "desktop_sandbox_unavailable"
            | "desktop_runtime_invalid"
            | "desktop_runtime_missing"
            | "desktop_home_unavailable"
            | "desktop_backend_conflict"
            | "desktop_sandbox_conflict"
            | "desktop_command_unsupported"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("ivnc-launch-{}", crate::remote::token()));
            fs::create_dir(&dir).unwrap();
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
            Self(dir)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn validates_the_session_socket_home_and_runtime_permissions() {
        let dir = Temp::new();
        let socket = dir.0.join("wayland-test");
        let _listener = UnixListener::bind(&socket).unwrap();
        let mut env = vec![
            ("XDG_RUNTIME_DIR".into(), dir.0.as_os_str().to_owned()),
            ("WAYLAND_DISPLAY".into(), "wayland-test".into()),
        ];
        validate_session(&dir.0, &env).unwrap();
        env[1].1 = socket.into_os_string();
        validate_session(&dir.0, &env).unwrap();
        fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            validate_session(&dir.0, &env).unwrap_err().code,
            "desktop_runtime_invalid"
        );
        fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            validate_session(&dir.0.join("absent"), &env)
                .unwrap_err()
                .code,
            "desktop_home_unavailable"
        );
        env[1].1 = "missing".into();
        assert_eq!(
            validate_session(&dir.0, &env).unwrap_err().code,
            "desktop_display_unavailable"
        );
        env.pop();
        assert_eq!(
            validate_session(&dir.0, &env).unwrap_err().code,
            "desktop_display_missing"
        );
        assert_eq!(
            validate_session(&dir.0, &[]).unwrap_err().code,
            "desktop_runtime_missing"
        );
    }

    #[test]
    fn diagnoses_only_this_attempt_and_ignores_nonfatal_warnings() {
        let dir = Temp::new();
        let log = dir.0.join("app.log");
        let root = "Running as root without --no-sandbox is not supported";
        fs::write(&log, root).unwrap();
        let cursor = LogCursor::new(log.clone());
        assert!(cursor.failure().is_none());
        writeln!(
            OpenOptions::new().append(true).open(&log).unwrap(),
            "\nERROR: Failed to connect to the bus"
        )
        .unwrap();
        assert!(cursor.failure().is_none());
        writeln!(
            OpenOptions::new().append(true).open(&log).unwrap(),
            "{root}"
        )
        .unwrap();
        let error = cursor.failure().unwrap();
        assert_eq!(error.code, "desktop_root_sandbox");
        assert!(permanent(&error));
        fs::rename(&log, dir.0.join("old")).unwrap();
        fs::write(&log, "Failed to initialize Wayland platform").unwrap();
        let error = cursor.failure().unwrap();
        assert_eq!(error.code, "desktop_wayland_connect");
        assert!(!permanent(&error));
        fs::remove_file(&log).unwrap();
        std::os::unix::fs::symlink(dir.0.join("old"), &log).unwrap();
        assert!(cursor.failure().is_none());
    }
}
