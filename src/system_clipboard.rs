//! System clipboard integration (Wayland via wl-clipboard).

use log::warn;
use std::io::Write;
use std::process::{Command, Stdio};

pub fn write(mime_type: &str, data: &[u8]) -> bool {
    let mut child = match Command::new("wl-copy")
        .arg("--type")
        .arg(mime_type)
        .stdin(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            warn!("wl-copy spawn failed: {}", err);
            return false;
        }
    };
    if let Some(stdin) = child.stdin.as_mut() {
        if stdin.write_all(data).is_err() {
            warn!("wl-copy write failed");
        }
    }
    let _ = child.wait();
    true
}
