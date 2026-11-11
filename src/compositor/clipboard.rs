//! Selection snapshots and ordered paste injection on the compositor thread.
use super::Compositor;
use crate::clipboard::paste::{PasteCommand, Shortcut};
use smithay::{
    backend::input::KeyState,
    input::keyboard::{FilterResult, Keycode},
    utils::SERIAL_COUNTER,
    wayland::selection::data_device::set_data_device_selection,
};
use std::{
    io::Write,
    os::fd::{AsRawFd, OwnedFd},
    sync::{Arc, Mutex, OnceLock, Weak},
    time::Duration,
};

/// A seat has one selection, even when several WebRTC clients control it.
/// Refuse overlapping transactions until the previous offer was delivered.
#[derive(Default)]
pub struct PasteGate {
    receipt: Weak<Mutex<Option<PasteCommand>>>,
}

impl PasteGate {
    fn busy(&self) -> bool {
        self.receipt.upgrade().is_some_and(|receipt| {
            receipt
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .is_some_and(PasteCommand::is_valid)
        })
    }

    fn track(&mut self, receipt: &Arc<Mutex<Option<PasteCommand>>>) {
        self.receipt = Arc::downgrade(receipt);
    }
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub bytes: Arc<[u8]>,
    pub revision: u64,
    receipt: Arc<Mutex<Option<PasteCommand>>>,
}

impl Snapshot {
    fn complete(&self, delivered: bool) {
        if let Some(command) = self
            .receipt
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            log::debug!(
                "Clipboard revision {} delivered={} request={}",
                self.revision,
                delivered,
                command.id
            );
            command.respond(
                if delivered {
                    "applied"
                } else {
                    "delivery_failed"
                },
                Some(self.revision),
            );
        }
    }
}

impl Compositor {
    /// Reserve the actual surface in input order before transferring the body.
    /// Registry indices can be reused when a window closes; the WlSurface cannot.
    pub fn reserve_paste(&self, command: &PasteCommand) {
        if !command.is_valid() {
            return;
        }
        let focus = self
            .seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus());
        if command
            .target
            .is_some_and(|target| self.window_registry.get(target as usize) != focus.as_ref())
        {
            return;
        }
        *command.focus.lock().unwrap_or_else(|e| e.into_inner()) = focus;
    }

    pub fn install_clipboard(&mut self, bytes: Arc<[u8]>, receipt: Option<PasteCommand>) -> u64 {
        self.clipboard_revision += 1;
        self.clipboard_pending_mime = None;
        self.clipboard_read_fd = None;
        let is_paste = receipt.is_some();
        let receipt = Arc::new(Mutex::new(receipt));
        if is_paste {
            self.clipboard_paste.track(&receipt);
        }
        set_data_device_selection(
            &self.display_handle,
            &self.seat,
            vec![
                "text/plain;charset=utf-8".into(),
                "text/plain".into(),
                "UTF8_STRING".into(),
            ],
            Snapshot {
                bytes,
                revision: self.clipboard_revision,
                receipt,
            },
        );
        self.clipboard_revision
    }

    /// Returns the installed revision. The delivery acknowledgement is sent by
    /// send_snapshot after EOF, not by this keyboard dispatch method.
    pub fn paste(&mut self, command: &PasteCommand) -> Result<u64, &'static str> {
        if !command.is_valid() {
            return Err("cancelled");
        }
        let keyboard = self.seat.get_keyboard().ok_or("no_focus")?;
        let focus = keyboard.current_focus().ok_or("no_focus")?;
        if command
            .focus
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            != Some(&focus)
        {
            return Err("focus_changed");
        }
        if self.clipboard_paste.busy() {
            return Err("busy");
        }
        let revision = self.install_clipboard(command.text.clone(), Some(command.clone()));
        log::debug!(
            "Dispatching paste request={} revision={}",
            command.id,
            revision
        );
        self.display_handle.flush_clients().ok();
        self.paste_shortcut(command.shortcut);
        self.display_handle.flush_clients().ok();
        Ok(revision)
    }

    pub fn paste_shortcut(&mut self, shortcut: Shortcut) {
        let keyboard = self.seat.get_keyboard().unwrap();
        let held = keyboard.pressed_keys();
        let (modifiers, key): (&[u32], u32) = match shortcut {
            Shortcut::CtrlV => (&[37], 55),
            Shortcut::CtrlShiftV => (&[37, 50], 55),
            Shortcut::ShiftInsert => (&[50], 118),
        };
        // Preserve real held keys, even if physical Ctrl was released while the
        // browser was acquiring/transferring the clipboard. Do not leak Alt/Meta.
        let changed: Vec<_> = [37, 105, 50, 62, 64, 108, 133, 134]
            .into_iter()
            .filter_map(|code| {
                let was_down = held.contains(&Keycode::from(code));
                (was_down != modifiers.contains(&code)).then_some((code, was_down))
            })
            .collect();
        let time = (self.start_time.elapsed().as_millis() & 0xFFFFFFFF) as u32;
        let mut send = |code: u32, down| {
            keyboard.input::<(), _>(
                self,
                Keycode::from(code),
                if down {
                    KeyState::Pressed
                } else {
                    KeyState::Released
                },
                SERIAL_COUNTER.next_serial(),
                time,
                |_, _, _| FilterResult::Forward,
            )
        };
        for &(code, was_down) in &changed {
            send(code, !was_down);
        }
        if held.contains(&Keycode::from(key)) {
            send(key, false);
        }
        send(key, true);
        send(key, false);
        if held.contains(&Keycode::from(key)) {
            send(key, true);
        }
        for &(code, was_down) in changed.iter().rev() {
            send(code, was_down);
        }
    }
}

/// Wayland clients may read old offers after a new selection is installed. The
/// bytes belong to the offer, never to a mutable global "pending paste".
pub fn send_snapshot(fd: OwnedFd, snapshot: Snapshot) {
    log::debug!(
        "Serving clipboard revision={} bytes={}",
        snapshot.revision,
        snapshot.bytes.len()
    );
    static WRITERS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    let Ok(permit) = WRITERS
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(64)))
        .clone()
        .try_acquire_owned()
    else {
        snapshot.complete(false);
        return;
    };
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
    if flags < 0
        || unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        snapshot.complete(false);
        return;
    }
    // The compositor runs on a synchronous thread, outside the web server's
    // Tokio runtime. A single bounded writer runtime serves all selection FDs.
    static RUNTIME: OnceLock<Option<tokio::runtime::Runtime>> = OnceLock::new();
    let Some(runtime) = RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("clipboard-writer")
            .enable_all()
            .build()
            .ok()
    }) else {
        snapshot.complete(false);
        return;
    };
    runtime.spawn(async move {
        let _permit = permit;
        let Ok(file) = tokio::io::unix::AsyncFd::new(std::fs::File::from(fd)) else {
            snapshot.complete(false);
            return;
        };
        let write = async {
            let mut remaining = snapshot.bytes.as_ref();
            while !remaining.is_empty() {
                let mut ready = file.writable().await?;
                match ready.try_io(|fd| fd.get_ref().write(remaining)) {
                    Ok(Ok(0)) => return Err(std::io::Error::from(std::io::ErrorKind::WriteZero)),
                    Ok(Ok(n)) => remaining = &remaining[n..],
                    Ok(Err(e)) => return Err(e),
                    Err(_) => continue,
                }
            }
            Ok::<_, std::io::Error>(())
        };
        let delivered = matches!(
            tokio::time::timeout(Duration::from_secs(10), write).await,
            Ok(Ok(()))
        );
        // Publish EOF before acknowledging: Chromium can coalesce clipboard reads
        // while a previous paste is still waiting for its pipe to close.
        drop(file);
        snapshot.complete(delivered);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::FromRawFd;
    use tokio::io::AsyncReadExt;

    #[test]
    fn shared_seat_stays_busy_until_delivery_cancellation_or_expiry() {
        let (reply, _) = tokio::sync::mpsc::unbounded_channel();
        let command = PasteCommand {
            id: 1,
            text: Arc::from(&b"A"[..]),
            shortcut: Shortcut::CtrlV,
            target: None,
            focus: Arc::new(Mutex::new(None)),
            valid: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            started: std::time::Instant::now(),
            reply,
        };
        let snapshot = Snapshot {
            bytes: command.text.clone(),
            revision: 1,
            receipt: Arc::new(Mutex::new(Some(command.clone()))),
        };
        let mut gate = PasteGate::default();
        gate.track(&snapshot.receipt);
        assert!(gate.busy());
        snapshot.complete(true);
        assert!(!gate.busy());

        *snapshot.receipt.lock().unwrap() = Some(command.clone());
        assert!(gate.busy());
        command
            .valid
            .store(false, std::sync::atomic::Ordering::Release);
        assert!(!gate.busy());
        command
            .valid
            .store(true, std::sync::atomic::Ordering::Release);
        snapshot.receipt.lock().unwrap().as_mut().unwrap().started -=
            crate::clipboard::paste::TRANSFER_TIMEOUT;
        assert!(!gate.busy());
        drop(snapshot);
        assert!(!gate.busy());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn old_offers_keep_their_bytes_and_large_writes_do_not_block() {
        let mut fds = [-1; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let read = unsafe { OwnedFd::from_raw_fd(fds[0]) };
        let write = unsafe { OwnedFd::from_raw_fd(fds[1]) };
        let old: Arc<[u8]> = vec![b'A'; 512 * 1024].into();
        let mut current = old.clone();
        let (reply, mut replies) = tokio::sync::mpsc::unbounded_channel();
        let command = PasteCommand {
            id: 1,
            text: old.clone(),
            shortcut: Shortcut::CtrlV,
            target: None,
            focus: Arc::new(Mutex::new(None)),
            valid: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            started: std::time::Instant::now(),
            reply,
        };
        let snapshot = Snapshot {
            bytes: current.clone(),
            revision: 1,
            receipt: Arc::new(Mutex::new(Some(command))),
        };
        let (returned, result) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            send_snapshot(write, snapshot);
            returned.send(()).unwrap();
        });
        result.recv_timeout(Duration::from_secs(2)).unwrap();
        thread.join().unwrap();
        assert!(
            replies.try_recv().is_err(),
            "ack must wait for data delivery"
        );
        current = Arc::from(&b"B"[..]);
        assert_eq!(&*current, b"B");
        // A pipe has far less capacity than this payload; a synchronous writer
        // would prevent us from ever reaching the read below.
        let mut reader = tokio::fs::File::from_std(std::fs::File::from(read));
        let mut output = Vec::new();
        tokio::time::timeout(Duration::from_secs(3), reader.read_to_end(&mut output))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(output.as_slice(), old.as_ref());
        let reply = tokio::time::timeout(Duration::from_secs(2), replies.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(reply.contains("applied"));
        assert!(replies.try_recv().is_err());
    }
}
