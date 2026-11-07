//! File upload handling for WebRTC and WebSocket data channels.

use crate::config::Config;
use log::{error, info, warn};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Component, PathBuf};

#[derive(Debug, Clone)]
pub struct FileUploadSettings {
    pub upload_dir: Option<PathBuf>,
    pub allow_upload: bool,
}

impl FileUploadSettings {
    pub fn from_config(config: &Config) -> Self {
        let allow_upload = config
            .input
            .file_transfers
            .iter()
            .any(|entry| entry.trim().eq_ignore_ascii_case("upload"));
        let upload_dir = if allow_upload {
            resolve_upload_dir(&config.input.upload_dir)
        } else {
            None
        };

        Self {
            upload_dir,
            allow_upload,
        }
    }
}

pub struct FileUploadHandler {
    settings: FileUploadSettings,
    active_path: Option<PathBuf>,
    active_file: Option<File>,
    active_parent: Option<File>,
    active_name: Option<std::ffi::CString>,
    expected_size: Option<u64>,
    written_size: u64,
}

impl FileUploadHandler {
    pub fn new(settings: FileUploadSettings) -> Self {
        Self {
            settings,
            active_path: None,
            active_file: None,
            active_parent: None,
            active_name: None,
            expected_size: None,
            written_size: 0,
        }
    }

    pub fn handle_control_message(&mut self, message: &str) -> bool {
        if message.starts_with("FILE_UPLOAD_START:") {
            if !self.is_upload_allowed() {
                warn!("File upload requested but uploads are disabled");
                return true;
            }
            let payload = message.trim_start_matches("FILE_UPLOAD_START:");
            let mut parts = payload.splitn(2, ':');
            let rel_path = parts.next().unwrap_or_default();
            let size = parts.next().unwrap_or_default();
            if let Err(err) = self.start_upload(rel_path, size) {
                error!("File upload start failed: {}", err);
                self.abort_active();
            }
            return true;
        }

        if message.starts_with("FILE_UPLOAD_END:") {
            let payload = message.trim_start_matches("FILE_UPLOAD_END:");
            info!("Received FILE_UPLOAD_END for {}", payload);
            self.finish_upload();
            return true;
        }

        if message.starts_with("FILE_UPLOAD_ERROR:") {
            let payload = message.trim_start_matches("FILE_UPLOAD_ERROR:");
            error!("Client reported upload error: {}", payload);
            self.abort_active();
            return true;
        }

        false
    }

    pub fn handle_binary(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        if data[0] != 0x01 {
            return;
        }
        let payload = &data[1..];
        if let Some(file) = self.active_file.as_mut() {
            if let Some(expected) = self.expected_size {
                let next = self.written_size.saturating_add(payload.len() as u64);
                if next > expected {
                    error!(
                        "Upload exceeded declared size (expected {}, got {})",
                        expected, next
                    );
                    self.abort_active();
                    return;
                }
            }
            if let Err(err) = file.write_all(payload) {
                error!(
                    "File write error for {:?}: {}",
                    self.active_path.as_ref().map(|p| p.as_path()),
                    err
                );
                self.abort_active();
                return;
            }
            self.written_size = self.written_size.saturating_add(payload.len() as u64);
        } else {
            warn!("Received file data after upload path is closed");
        }
    }

    pub fn abort_active(&mut self) {
        if let Some(mut file) = self.active_file.take() {
            let _ = file.flush();
        }
        self.remove_active_file();
        self.active_path = None;
        self.expected_size = None;
        self.written_size = 0;
    }

    pub fn finish_upload(&mut self) {
        if let Some(mut file) = self.active_file.take() {
            if let Err(err) = file.flush() {
                warn!("Failed to flush upload file: {}", err);
            }
        }
        if let Some(path) = self.active_path.take() {
            if let Some(expected) = self.expected_size {
                if self.written_size != expected {
                    warn!(
                        "Upload size mismatch for {:?}: expected {}, got {}",
                        path, expected, self.written_size
                    );
                    self.remove_active_file();
                } else {
                    info!("Upload finished: {:?}", path);
                }
            } else {
                info!("Upload finished: {:?}", path);
            }
        }
        self.active_parent = None;
        self.active_name = None;
        self.expected_size = None;
        self.written_size = 0;
    }

    fn remove_active_file(&mut self) {
        use std::os::fd::AsRawFd;
        if let (Some(parent), Some(name)) = (self.active_parent.take(), self.active_name.take()) {
            // The directory fd pins the validated parent even if the app renames it.
            unsafe {
                libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0);
            }
        }
    }

    fn is_upload_allowed(&self) -> bool {
        self.settings.allow_upload && self.settings.upload_dir.is_some()
    }

    fn start_upload(&mut self, rel_path: &str, size_str: &str) -> Result<(), String> {
        let upload_dir = self
            .settings
            .upload_dir
            .as_ref()
            .ok_or_else(|| "Upload directory is not configured".to_string())?;

        let size = size_str
            .trim()
            .parse::<u64>()
            .map_err(|_| "Invalid file size")?;
        if size == 0 {
            return Err("Invalid file size".to_string());
        }
        const MAX_UPLOAD_BYTES: u64 = 512 * 1024 * 1024;
        if size > MAX_UPLOAD_BYTES {
            return Err(format!(
                "Upload exceeds size limit ({} bytes)",
                MAX_UPLOAD_BYTES
            ));
        }

        let safe_rel = sanitize_relative_path(rel_path)
            .ok_or_else(|| format!("Invalid relative path: {}", rel_path))?;

        let upload_dir = upload_dir.clone();
        if self.active_file.is_some() {
            self.finish_upload();
        }
        let target_path = upload_dir.join(&safe_rel);
        let (parent, name, file) = open_upload(&upload_dir, &safe_rel)
            .map_err(|e| format!("Cannot create upload: {e}"))?;
        self.active_parent = Some(parent);
        self.active_name = Some(name);
        self.active_file = Some(file);
        self.active_path = Some(target_path.clone());
        self.expected_size = Some(size);
        self.written_size = 0;
        info!("Upload started: {:?}", target_path);
        Ok(())
    }
}

fn resolve_upload_dir(raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed == "/sys" || trimmed == "/proc" || trimmed == "/dev" {
        warn!("Refusing to use upload directory {}", trimmed);
        return None;
    }
    let expanded = if trimmed == "~/Desktop" || trimmed.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            let suffix = trimmed.trim_start_matches("~/");
            PathBuf::from(home).join(suffix)
        } else {
            PathBuf::from(trimmed)
        }
    } else {
        PathBuf::from(trimmed)
    };

    // Defer create_dir_all until an actual upload starts.
    Some(expanded)
}

fn sanitize_relative_path(rel_path: &str) -> Option<PathBuf> {
    let trimmed = rel_path.trim().trim_start_matches(&['/', '\\'][..]);
    if trimmed.is_empty() {
        return None;
    }

    let normalized = trimmed.replace('\\', "/");
    let mut safe = PathBuf::new();
    for component in PathBuf::from(normalized).components() {
        match component {
            Component::Normal(part) => safe.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }

    if safe.as_os_str().is_empty() {
        None
    } else {
        Some(safe)
    }
}

fn open_upload(
    root: &std::path::Path,
    relative: &std::path::Path,
) -> std::io::Result<(File, std::ffi::CString, File)> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::{
                ffi::OsStrExt,
                fs::{MetadataExt, OpenOptionsExt},
            },
        },
    };
    fs::create_dir_all(root)?;
    let mut parent = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root)?;
    let parts: Vec<_> = relative.components().collect();
    for (index, part) in parts.iter().enumerate() {
        let Component::Normal(part) = part else {
            return Err(std::io::ErrorKind::InvalidInput.into());
        };
        let name = CString::new(part.as_bytes()).map_err(|_| std::io::ErrorKind::InvalidInput)?;
        if index + 1 == parts.len() {
            let fd = unsafe {
                libc::openat(
                    parent.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_WRONLY
                        | libc::O_CREAT
                        | libc::O_NOFOLLOW
                        | libc::O_CLOEXEC
                        | libc::O_NONBLOCK,
                    0o600,
                )
            };
            if fd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let file = unsafe { File::from_raw_fd(fd) };
            let meta = file.metadata()?;
            if !meta.is_file() || meta.nlink() != 1 {
                return Err(std::io::ErrorKind::PermissionDenied.into());
            }
            file.set_len(0)?;
            return Ok((parent, name, file));
        }
        if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } != 0
            && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
        {
            return Err(std::io::Error::last_os_error());
        }
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        parent = unsafe { File::from_raw_fd(fd) };
    }
    Err(std::io::ErrorKind::InvalidInput.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upload_rejects_symlinks_and_hardlinks_and_keeps_renamed_parent() {
        let root = std::env::temp_dir().join(format!("ivnc-upload-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("downloads")).unwrap();
        fs::write(root.join("secret"), b"secret").unwrap();
        std::os::unix::fs::symlink(&root, root.join("downloads/out")).unwrap();
        fs::hard_link(root.join("secret"), root.join("downloads/link")).unwrap();
        for name in ["out/secret", "link", "../secret"] {
            assert!(open_upload(&root.join("downloads"), std::path::Path::new(name)).is_err());
        }
        let (_dir, _name, mut file) =
            open_upload(&root.join("downloads"), std::path::Path::new("nested/ok")).unwrap();
        fs::rename(root.join("downloads/nested"), root.join("saved")).unwrap();
        std::os::unix::fs::symlink(&root, root.join("downloads/nested")).unwrap();
        file.write_all(b"hello").unwrap();
        assert_eq!(fs::read(root.join("saved/ok")).unwrap(), b"hello");
        assert_eq!(fs::read(root.join("secret")).unwrap(), b"secret");
        fs::remove_dir_all(root).unwrap();
    }
}

impl Drop for FileUploadHandler {
    fn drop(&mut self) {
        self.abort_active();
    }
}
