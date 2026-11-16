//! Optional, connection-local receipts for browser IME commits. Legacy `t,`
//! input remains supported; new clients use this only when advertised.
use super::{InputEvent, InputEventData};
use serde::Deserialize;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

pub const TIMEOUT: Duration = Duration::from_secs(2);
pub const MAX_BYTES: usize = 48 * 1024;

#[derive(Clone, Debug)]
pub struct TextCommit {
    pub id: u64,
    pub focus: uuid::Uuid,
    pub valid: Arc<AtomicBool>,
    pub started: Instant,
    reply: mpsc::UnboundedSender<String>,
}

impl TextCommit {
    pub fn is_valid(&self) -> bool {
        self.valid.load(Ordering::Acquire) && self.started.elapsed() < TIMEOUT
    }

    pub fn respond(&self, status: &str) {
        if self.valid.swap(false, Ordering::AcqRel) {
            respond(&self.reply, self.id, status);
        }
    }
}

fn respond(reply: &mpsc::UnboundedSender<String>, id: u64, status: &str) {
    let _ = reply.send(
        serde_json::json!({"type":"text_input_result", "data": {
            "id":id, "status":status
        }})
        .to_string(),
    );
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Commit {
        id: u64,
        focus: Option<String>,
        text: String,
    },
    Cancel {
        id: u64,
    },
}

pub struct TextCommitReceiver {
    input: mpsc::UnboundedSender<InputEventData>,
    reply: mpsc::UnboundedSender<String>,
    highest_id: u64,
    pending: HashMap<u64, TextCommit>,
}

impl TextCommitReceiver {
    pub fn new(
        input: mpsc::UnboundedSender<InputEventData>,
        reply: mpsc::UnboundedSender<String>,
    ) -> Self {
        Self {
            input,
            reply,
            highest_id: 0,
            pending: HashMap::new(),
        }
    }

    pub fn expire(&mut self) {
        self.pending.retain(|_, commit| {
            if commit.started.elapsed() >= TIMEOUT {
                commit.respond("timeout");
            }
            commit.is_valid()
        });
    }

    pub fn handle(&mut self, message: &str) -> bool {
        let Some(body) = message.strip_prefix("ti,") else {
            return false;
        };
        self.expire();
        // Bound parsing too, including malicious JSON escaping/oversized input.
        if body.len() > MAX_BYTES * 6 + 256 {
            return true;
        }
        let Ok(request) = serde_json::from_str::<Request>(body) else {
            return true;
        };
        match request {
            Request::Cancel { id } => {
                // A cancellation may overtake text still waiting behind a
                // browser clipboard transfer. Fence that ID before it arrives.
                if id > 0 && id <= 9_007_199_254_740_991 {
                    self.highest_id = self.highest_id.max(id);
                }
                if let Some(commit) = self.pending.remove(&id) {
                    commit.respond("cancelled");
                }
            }
            Request::Commit { id, focus, text } => {
                if id <= self.highest_id || id > 9_007_199_254_740_991 {
                    respond(&self.reply, id, "invalid");
                    return true;
                }
                self.highest_id = id;
                if self.pending.len() >= 32 || text.len() > MAX_BYTES {
                    respond(&self.reply, id, "limit");
                    return true;
                }
                if text.is_empty() || text.contains('\0') {
                    respond(&self.reply, id, "invalid");
                    return true;
                }
                let Some(focus) = focus.and_then(|s| uuid::Uuid::parse_str(&s).ok()) else {
                    respond(&self.reply, id, "no_focus");
                    return true;
                };
                let commit = TextCommit {
                    id,
                    focus,
                    valid: Arc::new(AtomicBool::new(true)),
                    started: Instant::now(),
                    reply: self.reply.clone(),
                };
                self.pending.insert(id, commit.clone());
                if self
                    .input
                    .send(InputEventData {
                        event_type: InputEvent::TextInput,
                        text,
                        text_commit: Some(commit.clone()),
                        ..Default::default()
                    })
                    .is_err()
                {
                    commit.respond("disconnected");
                }
            }
        }
        true
    }
}

impl Drop for TextCommitReceiver {
    fn drop(&mut self) {
        for commit in self.pending.values() {
            commit.valid.store(false, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn status(rx: &mut mpsc::UnboundedReceiver<String>) -> String {
        let value: serde_json::Value = serde_json::from_str(&rx.try_recv().unwrap()).unwrap();
        assert_eq!(value["type"], "text_input_result");
        assert!(value["data"].get("text").is_none());
        value["data"]["status"].as_str().unwrap().into()
    }

    #[test]
    fn commits_are_bounded_fenced_cancellable_and_never_replayed() {
        let (input, mut events) = mpsc::unbounded_channel();
        let (reply, mut results) = mpsc::unbounded_channel();
        let mut receiver = TextCommitReceiver::new(input, reply);
        let focus = uuid::Uuid::new_v4();
        let message = format!(
            "ti,{}",
            serde_json::json!({"type":"commit","id":1,"focus":focus.to_string(),"text":"中文,🙂"})
        );
        assert!(!receiver.handle("t,legacy"));
        assert!(receiver.handle(&message));
        let event = events.try_recv().unwrap();
        assert_eq!(event.text, "中文,🙂");
        let commit = event.text_commit.unwrap();
        assert_eq!(commit.focus, focus);
        assert!(commit.is_valid());
        receiver.handle("ti,{\"type\":\"cancel\",\"id\":1}");
        assert!(!commit.is_valid());
        assert_eq!(status(&mut results), "cancelled");
        commit.respond("dispatched");
        assert!(results.try_recv().is_err());
        receiver.handle(&message);
        assert_eq!(status(&mut results), "invalid");
        assert!(events.try_recv().is_err());
        receiver.handle("ti,{\"type\":\"commit\",\"id\":2,\"focus\":null,\"text\":\"中文\"}");
        assert_eq!(status(&mut results), "no_focus");
        receiver.handle(&format!("ti,{}", serde_json::json!({"type":"commit","id":3,"focus":focus.to_string(),"text":"a".repeat(MAX_BYTES+1)})));
        assert_eq!(status(&mut results), "limit");
    }

    #[test]
    fn expiry_and_disconnect_invalidate_queued_input() {
        let (input, mut events) = mpsc::unbounded_channel();
        let (reply, mut results) = mpsc::unbounded_channel();
        let mut receiver = TextCommitReceiver::new(input, reply);
        let message = |id| {
            format!(
                "ti,{}",
                serde_json::json!({"type":"commit","id":id,"focus":uuid::Uuid::new_v4().to_string(),"text":"secret"})
            )
        };
        receiver.handle(&message(1));
        receiver.pending.get_mut(&1).unwrap().started -= TIMEOUT;
        receiver.expire();
        assert!(!events.try_recv().unwrap().text_commit.unwrap().is_valid());
        assert_eq!(status(&mut results), "timeout");
        receiver.handle(&message(2));
        drop(receiver);
        assert!(!events.try_recv().unwrap().text_commit.unwrap().is_valid());
    }

    #[test]
    fn early_cancellation_fences_delayed_text_and_sessions_have_separate_receipts() {
        let (input, mut events) = mpsc::unbounded_channel();
        let (reply_a, mut results_a) = mpsc::unbounded_channel();
        let (reply_b, mut results_b) = mpsc::unbounded_channel();
        let mut a = TextCommitReceiver::new(input.clone(), reply_a);
        let mut b = TextCommitReceiver::new(input, reply_b);
        let message = format!(
            "ti,{}",
            serde_json::json!({"type":"commit","id":1,"focus":uuid::Uuid::new_v4().to_string(),"text":"中文"})
        );
        a.handle("ti,{\"type\":\"cancel\",\"id\":1}");
        a.handle(&message);
        assert_eq!(status(&mut results_a), "invalid");
        assert!(events.try_recv().is_err());
        b.handle(&message);
        events
            .try_recv()
            .unwrap()
            .text_commit
            .unwrap()
            .respond("dispatched");
        assert_eq!(status(&mut results_b), "dispatched");
        assert!(results_a.try_recv().is_err());
    }
}
