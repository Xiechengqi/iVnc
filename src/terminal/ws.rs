use crate::web::shared::SharedState;
use axum::extract::ws::{Message, WebSocket};
use base64::Engine;
use futures::{SinkExt, StreamExt};
use log::{debug, error, info, warn};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{mpsc as std_mpsc, Arc, Mutex};
use tokio::sync::mpsc;

const DEFAULT_COLS: u16 = 120;
const DEFAULT_ROWS: u16 = 32;

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "lowercase")]
enum ClientMessage {
    Input { data: String },
    Resize { cols: u16, rows: u16 },
    Ping,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
#[serde(rename_all = "lowercase")]
enum ServerMessage {
    Ready { shell: String },
    Output { data: String },
    Exit { code: Option<i32> },
    Error { message: String },
    Pong,
}

enum PtyEvent {
    Output(Vec<u8>),
    Exit(Option<i32>),
    Error(String),
}

struct TerminalSession {
    shell: String,
    writer_tx: std_mpsc::Sender<String>,
    master: Arc<Mutex<Box<dyn portable_pty::MasterPty + Send>>>,
    child_killer: Arc<Mutex<Option<Box<dyn portable_pty::ChildKiller + Send + Sync>>>>,
    events: mpsc::UnboundedReceiver<PtyEvent>,
}

impl TerminalSession {
    fn spawn(state: &SharedState) -> Result<Self, String> {
        let shell = resolve_shell(&state.config.terminal.shell)?;
        let cwd = resolve_cwd(&state.config.terminal.cwd);
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: DEFAULT_ROWS,
                cols: DEFAULT_COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|err| format!("failed to open pty: {}", err))?;

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|err| format!("failed to clone pty reader: {}", err))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|err| format!("failed to take pty writer: {}", err))?;
        let master = Arc::new(Mutex::new(pair.master));

        let mut command = CommandBuilder::new(&shell);
        configure_interactive_shell(&mut command, &shell);
        command.env("TERM", &state.config.terminal.term);
        command.cwd(cwd);

        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(|err| format!("failed to spawn shell '{}': {}", shell, err))?;
        let child_killer = Arc::new(Mutex::new(Some(child.clone_killer())));
        drop(pair.slave);

        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let output_tx = events_tx.clone();
        tokio::task::spawn_blocking(move || {
            let mut buf = [0_u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if output_tx.send(PtyEvent::Output(buf[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                    Err(err) => {
                        let _ =
                            output_tx.send(PtyEvent::Error(format!("pty read failed: {}", err)));
                        break;
                    }
                }
            }
        });

        let (writer_tx, writer_rx) = std_mpsc::channel::<String>();
        let writer_events_tx = events_tx.clone();
        tokio::task::spawn_blocking(move || {
            let mut writer = writer;
            while let Ok(data) = writer_rx.recv() {
                if let Err(err) = writer.write_all(data.as_bytes()) {
                    let _ = writer_events_tx
                        .send(PtyEvent::Error(format!("pty write failed: {}", err)));
                    break;
                }
                if let Err(err) = writer.flush() {
                    let _ = writer_events_tx
                        .send(PtyEvent::Error(format!("pty flush failed: {}", err)));
                    break;
                }
            }
        });

        let wait_child_killer = child_killer.clone();
        tokio::task::spawn_blocking(move || {
            let status = match child.wait() {
                Ok(status) => Some(status.exit_code() as i32),
                Err(err) => {
                    let _ = events_tx.send(PtyEvent::Error(format!("shell wait failed: {}", err)));
                    None
                }
            };
            if let Ok(mut killer) = wait_child_killer.lock() {
                *killer = None;
            }
            let _ = events_tx.send(PtyEvent::Exit(status));
        });

        Ok(Self {
            shell,
            writer_tx,
            master,
            child_killer,
            events: events_rx,
        })
    }

    fn write_input(&self, data: String) {
        let _ = self.writer_tx.send(data);
    }

    fn resize(&self, cols: u16, rows: u16) {
        let cols = cols.clamp(20, 400);
        let rows = rows.clamp(5, 160);
        if let Ok(master) = self.master.lock() {
            if let Err(err) = master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            }) {
                warn!("terminal resize failed: {}", err);
            }
        }
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        if let Ok(mut child_killer) = self.child_killer.lock() {
            if let Some(mut child_killer) = child_killer.take() {
                if let Err(err) = child_killer.kill() {
                    debug!("terminal child kill failed during cleanup: {}", err);
                }
            }
        }
    }
}

pub async fn handle_terminal_websocket(socket: WebSocket, state: Arc<SharedState>) {
    if !state.config.terminal.enabled {
        let mut socket = socket;
        let _ = send_json(
            &mut socket,
            &ServerMessage::Error {
                message: "terminal is disabled".to_string(),
            },
        )
        .await;
        return;
    }

    let session = match TerminalSession::spawn(&state) {
        Ok(session) => session,
        Err(err) => {
            let mut socket = socket;
            let _ = send_json(&mut socket, &ServerMessage::Error { message: err }).await;
            return;
        }
    };

    info!("terminal session started with shell {}", session.shell);
    run_terminal_socket(socket, session).await;
}

async fn run_terminal_socket(socket: WebSocket, mut session: TerminalSession) {
    let (mut sender, mut receiver) = socket.split();
    if send_split_json(
        &mut sender,
        &ServerMessage::Ready {
            shell: session.shell.clone(),
        },
    )
    .await
    .is_err()
    {
        return;
    }

    loop {
        tokio::select! {
            Some(event) = session.events.recv() => {
                let message = match event {
                    PtyEvent::Output(data) => {
                        let encoded = base64::engine::general_purpose::STANDARD.encode(data);
                        ServerMessage::Output { data: encoded }
                    }
                    PtyEvent::Exit(code) => ServerMessage::Exit { code },
                    PtyEvent::Error(message) => ServerMessage::Error { message },
                };
                let is_exit = matches!(message, ServerMessage::Exit { .. });
                if send_split_json(&mut sender, &message).await.is_err() {
                    return;
                }
                if is_exit {
                    return;
                }
            }
            incoming = receiver.next() => {
                let Some(Ok(message)) = incoming else {
                    return;
                };
                match message {
                    Message::Text(text) => handle_client_text(&session, text.as_str(), &mut sender).await,
                    Message::Binary(data) => {
                        debug!("ignoring terminal binary message of {} bytes", data.len());
                    }
                    Message::Close(_) => return,
                    Message::Ping(data) => {
                        let _ = sender.send(Message::Pong(data)).await;
                    }
                    Message::Pong(_) => {}
                }
            }
        }
    }
}

async fn handle_client_text(
    session: &TerminalSession,
    text: &str,
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
) {
    match serde_json::from_str::<ClientMessage>(text) {
        Ok(ClientMessage::Input { data }) => session.write_input(data),
        Ok(ClientMessage::Resize { cols, rows }) => session.resize(cols, rows),
        Ok(ClientMessage::Ping) => {
            let _ = send_split_json(sender, &ServerMessage::Pong).await;
        }
        Err(err) => {
            let _ = send_split_json(
                sender,
                &ServerMessage::Error {
                    message: format!("invalid terminal message: {}", err),
                },
            )
            .await;
        }
    }
}

async fn send_json(socket: &mut WebSocket, message: &ServerMessage) -> Result<(), ()> {
    let payload = serde_json::to_string(message).map_err(|err| {
        error!("failed to serialize terminal message: {}", err);
    })?;
    socket
        .send(Message::Text(payload.into()))
        .await
        .map_err(|err| {
            debug!("terminal websocket send failed: {}", err);
        })
}

async fn send_split_json(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    message: &ServerMessage,
) -> Result<(), ()> {
    let payload = serde_json::to_string(message).map_err(|err| {
        error!("failed to serialize terminal message: {}", err);
    })?;
    sender
        .send(Message::Text(payload.into()))
        .await
        .map_err(|err| {
            debug!("terminal websocket send failed: {}", err);
        })
}

fn resolve_shell(configured: &str) -> Result<String, String> {
    for candidate in [configured, "/bin/bash", "/bin/sh"] {
        if candidate.trim().is_empty() {
            continue;
        }
        if std::path::Path::new(candidate).exists() {
            return Ok(candidate.to_string());
        }
    }
    Err("no usable shell found".to_string())
}

fn configure_interactive_shell(command: &mut CommandBuilder, shell: &str) {
    let shell_name = std::path::Path::new(shell)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(shell);

    match shell_name {
        "bash" => command.args(["--noprofile", "--norc", "-i"]),
        "sh" | "dash" => command.arg("-i"),
        _ => {}
    }
}

fn resolve_cwd(configured: &str) -> PathBuf {
    let path = if configured == "~" {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from("/root"))
    } else if let Some(rest) = configured.strip_prefix("~/") {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/root"))
            .join(rest)
    } else {
        PathBuf::from(configured)
    };

    if path.is_dir() {
        path
    } else {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from("/root"))
    }
}
