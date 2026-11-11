//! A paste is one compositor command, not an unrelated clipboard write and key.
use super::MAX_CLIPBOARD_BYTES;
use crate::input::{InputEvent, InputEventData};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, Weak,
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

pub const TRANSFER_TIMEOUT: Duration = Duration::from_secs(30);
pub const CHUNK_BYTES: usize = 12 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Shortcut {
    CtrlV,
    CtrlShiftV,
    ShiftInsert,
}

#[derive(Clone, Debug)]
pub struct PasteCommand {
    pub id: u64,
    pub text: Arc<[u8]>,
    pub shortcut: Shortcut,
    pub target: Option<u32>,
    pub focus:
        Arc<Mutex<Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>>>,
    pub valid: Arc<AtomicBool>,
    pub started: Instant,
    pub reply: mpsc::UnboundedSender<String>,
}

impl PasteCommand {
    pub fn is_valid(&self) -> bool {
        self.valid.load(Ordering::Acquire) && self.started.elapsed() < TRANSFER_TIMEOUT
    }

    pub fn respond(&self, status: &str, revision: Option<u64>) {
        respond(&self.reply, self.id, status, revision);
    }
}

fn respond(reply: &mpsc::UnboundedSender<String>, id: u64, status: &str, revision: Option<u64>) {
    let _ = reply.send(format!(
        "paste_ack,{}",
        serde_json::json!({
            "id": id, "status": status, "revision": revision
        })
    ));
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Hello,
    Start {
        id: u64,
        size: usize,
        shortcut: Shortcut,
        target: Option<u32>,
    },
    Chunk {
        id: u64,
        data: String,
    },
    Commit {
        id: u64,
    },
    Cancel {
        id: u64,
    },
}

struct Transfer {
    command: PasteCommand,
    bytes: Vec<u8>,
    size: usize,
}

pub struct PasteReceiver {
    enabled: bool,
    input: mpsc::UnboundedSender<InputEventData>,
    reply: mpsc::UnboundedSender<String>,
    highest_id: u64,
    transfer: Option<Transfer>,
    queued: HashMap<u64, Weak<AtomicBool>>,
}

impl PasteReceiver {
    pub fn new(
        enabled: bool,
        input: mpsc::UnboundedSender<InputEventData>,
        reply: mpsc::UnboundedSender<String>,
    ) -> Self {
        Self {
            enabled,
            input,
            reply,
            highest_id: 0,
            transfer: None,
            queued: HashMap::new(),
        }
    }

    pub fn handle(&mut self, message: &str) -> bool {
        let Some(payload) = message.strip_prefix("paste,") else {
            return false;
        };
        // Bound JSON allocation too, before decoding the chunk.
        if payload.len() > CHUNK_BYTES * 2 {
            return true;
        }
        let Ok(request) = serde_json::from_str::<Request>(payload) else {
            respond(&self.reply, 0, "invalid_request", None);
            return true;
        };
        self.queued.retain(|_, valid| valid.strong_count() > 0);
        if self
            .transfer
            .as_ref()
            .is_some_and(|t| !t.command.is_valid())
        {
            self.abort("expired");
        }
        match request {
            Request::Hello => {
                let _ = self.reply.send(format!(
                    "clipboard_caps,{}",
                    serde_json::json!({
                        "version": 1, "enabled": self.enabled, "max_bytes": MAX_CLIPBOARD_BYTES,
                        "chunk_bytes": CHUNK_BYTES
                    })
                ));
            }
            Request::Start {
                id,
                size,
                shortcut,
                target,
            } => {
                if id == 0 || id <= self.highest_id {
                    respond(&self.reply, id, "duplicate", None);
                    return true;
                }
                self.highest_id = id; // Never replay an ID, including failed transfers.
                if !self.enabled || size > MAX_CLIPBOARD_BYTES || self.queued.len() >= 32 {
                    respond(
                        &self.reply,
                        id,
                        if !self.enabled { "disabled" } else { "limit" },
                        None,
                    );
                    return true;
                }
                self.abort("superseded");
                self.transfer = Some(Transfer {
                    command: PasteCommand {
                        id,
                        text: Arc::from([]),
                        shortcut,
                        target,
                        focus: Arc::new(Mutex::new(None)),
                        valid: Arc::new(AtomicBool::new(true)),
                        started: Instant::now(),
                        reply: self.reply.clone(),
                    },
                    bytes: Vec::new(),
                    size,
                });
                let command = self.transfer.as_ref().unwrap().command.clone();
                if self
                    .input
                    .send(InputEventData {
                        event_type: InputEvent::PasteStart,
                        paste: Some(command),
                        ..Default::default()
                    })
                    .is_err()
                {
                    self.abort("unavailable");
                }
            }
            Request::Chunk { id, data } => {
                if let Some(t) = self.transfer.as_mut().filter(|t| t.command.id == id) {
                    match STANDARD.decode(data) {
                        Ok(bytes)
                            if bytes.len() <= CHUNK_BYTES
                                && t.bytes.len().saturating_add(bytes.len()) <= t.size =>
                        {
                            t.bytes.extend(bytes)
                        }
                        _ => self.abort("invalid_chunk"),
                    }
                }
            }
            Request::Commit { id } => {
                if self.transfer.as_ref().is_some_and(|t| t.command.id == id) {
                    let mut t = self.transfer.take().unwrap();
                    if t.bytes.len() != t.size || std::str::from_utf8(&t.bytes).is_err() {
                        t.command.respond("invalid_content", None);
                    } else {
                        t.command.text = Arc::from(t.bytes);
                        self.queued.insert(id, Arc::downgrade(&t.command.valid));
                        if self
                            .input
                            .send(InputEventData {
                                event_type: InputEvent::Paste,
                                paste: Some(t.command),
                                ..Default::default()
                            })
                            .is_err()
                        {
                            respond(&self.reply, id, "unavailable", None);
                        }
                    }
                } else {
                    respond(&self.reply, id, "unknown_request", None);
                }
            }
            Request::Cancel { id } => {
                if self.transfer.as_ref().is_some_and(|t| t.command.id == id) {
                    self.abort("cancelled");
                }
                if let Some(valid) = self.queued.remove(&id).and_then(|v| v.upgrade()) {
                    valid.store(false, Ordering::Release);
                }
            }
        }
        true
    }

    fn abort(&mut self, status: &str) {
        if let Some(t) = self.transfer.take() {
            t.command.valid.store(false, Ordering::Release);
            t.command.respond(status, None);
        }
    }

    pub fn cancel_all(&mut self) {
        self.abort("cancelled");
        for valid in self.queued.drain().filter_map(|(_, v)| v.upgrade()) {
            valid.store(false, Ordering::Release);
        }
    }
}

impl Drop for PasteReceiver {
    fn drop(&mut self) {
        self.cancel_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn receiver(
        enabled: bool,
    ) -> (
        PasteReceiver,
        mpsc::UnboundedReceiver<InputEventData>,
        mpsc::UnboundedReceiver<String>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        let (reply, replies) = mpsc::unbounded_channel();
        (PasteReceiver::new(enabled, tx, reply), rx, replies)
    }
    fn next_paste(rx: &mut mpsc::UnboundedReceiver<InputEventData>) -> Option<PasteCommand> {
        while let Ok(event) = rx.try_recv() {
            if event.event_type == InputEvent::Paste {
                return event.paste;
            }
        }
        None
    }
    fn send(r: &mut PasteReceiver, value: serde_json::Value) {
        assert!(r.handle(&format!("paste,{value}")));
    }
    fn start(r: &mut PasteReceiver, id: u64, size: usize) {
        send(
            r,
            json!({"type":"start","id":id,"size":size,"shortcut":"ctrl-v","target":7}),
        );
    }
    fn complete(r: &mut PasteReceiver, id: u64, text: &[u8]) {
        start(r, id, text.len());
        send(
            r,
            json!({"type":"chunk","id":id,"data":STANDARD.encode(text)}),
        );
        send(r, json!({"type":"commit","id":id}));
    }

    #[test]
    fn paste_only_enters_input_queue_after_complete_utf8_payload() {
        let (mut r, mut rx, mut replies) = receiver(true);
        let text = "中文\nnew clipboard".as_bytes();
        start(&mut r, 1, text.len());
        send(
            &mut r,
            json!({"type":"chunk","id":1,"data":STANDARD.encode(&text[..2])}),
        );
        assert!(next_paste(&mut rx).is_none());
        send(
            &mut r,
            json!({"type":"chunk","id":1,"data":STANDARD.encode(&text[2..])}),
        );
        assert!(next_paste(&mut rx).is_none());
        send(&mut r, json!({"type":"commit","id":1}));
        let command = next_paste(&mut rx).unwrap();
        assert_eq!(&*command.text, text);
        assert_eq!(command.target, Some(7));
        assert!(
            replies.try_recv().is_err(),
            "receipt is not an applied acknowledgement"
        );
        command.respond("applied", Some(3));
        assert!(replies.try_recv().unwrap().contains("applied"));
    }

    #[test]
    fn repeated_id_never_pastes_twice_but_same_content_with_new_id_does() {
        let (mut r, mut rx, _) = receiver(true);
        complete(&mut r, 1, b"A");
        complete(&mut r, 1, b"B");
        complete(&mut r, 2, b"A");
        let first = next_paste(&mut rx).unwrap();
        let second = next_paste(&mut rx).unwrap();
        assert_eq!((first.id, second.id), (1, 2));
        assert_eq!(&*first.text, b"A");
        assert_eq!(&*second.text, b"A");
        assert!(next_paste(&mut rx).is_none());
    }

    #[test]
    fn cancel_and_connection_drop_invalidate_queued_operations() {
        let (mut r, mut rx, _) = receiver(true);
        complete(&mut r, 1, b"A");
        let first = next_paste(&mut rx).unwrap();
        send(&mut r, json!({"type":"cancel","id":1}));
        assert!(!first.is_valid());
        complete(&mut r, 2, b"B");
        let second = next_paste(&mut rx).unwrap();
        assert!(second.is_valid());
        drop(r);
        assert!(!second.is_valid());
    }

    #[test]
    fn invalid_or_incomplete_transfers_do_not_execute() {
        let (mut r, mut rx, _) = receiver(true);
        complete(&mut r, 1, &[255]);
        start(&mut r, 2, 2);
        send(&mut r, json!({"type":"chunk","id":2,"data":"YQ=="}));
        send(&mut r, json!({"type":"commit","id":2}));
        start(&mut r, 3, 1);
        send(&mut r, json!({"type":"chunk","id":3,"data":"YWI="}));
        send(&mut r, json!({"type":"commit","id":3}));
        start(&mut r, 4, MAX_CLIPBOARD_BYTES + 1);
        send(&mut r, json!({"type":"commit","id":4}));
        assert!(next_paste(&mut rx).is_none());
    }

    #[test]
    fn empty_text_is_valid_and_expired_transfers_are_discarded() {
        let (mut r, mut rx, _) = receiver(true);
        complete(&mut r, 1, b"");
        assert!(next_paste(&mut rx).unwrap().text.is_empty());
        start(&mut r, 2, 0);
        r.transfer.as_mut().unwrap().command.started = Instant::now() - TRANSFER_TIMEOUT;
        send(&mut r, json!({"type":"commit","id":2}));
        assert!(next_paste(&mut rx).is_none());
    }

    #[test]
    fn disabled_server_negotiates_without_accepting_paste() {
        let (mut r, mut rx, mut replies) = receiver(false);
        send(&mut r, json!({"type":"hello"}));
        assert!(replies.try_recv().unwrap().contains("\"enabled\":false"));
        complete(&mut r, 1, b"text");
        assert!(next_paste(&mut rx).is_none());
    }

    #[test]
    fn sessions_have_independent_ids_and_reply_channels() {
        let (mut a, mut arx, mut areplies) = receiver(true);
        let (mut b, mut brx, mut breplies) = receiver(true);
        complete(&mut a, 1, b"A");
        complete(&mut b, 1, b"B");
        arx.try_recv()
            .unwrap()
            .paste
            .unwrap()
            .respond("applied", Some(1));
        assert!(areplies.try_recv().is_ok());
        assert!(breplies.try_recv().is_err());
        assert_eq!(&*next_paste(&mut brx).unwrap().text, b"B");
    }
}
