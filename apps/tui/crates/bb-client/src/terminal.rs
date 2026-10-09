use std::collections::VecDeque;
use std::time::Duration;

use bb_wire::routes::terminal_ws_path;
use bb_wire::terminal::{
    TERMINAL_DATA_MAX_BYTES, TerminalClientMessage, TerminalServerMessage, TerminalWebSocketQuery,
};
use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;
use url::Url;

use crate::http::BbClient;
use crate::ws::{WsOpenError, WsStream, open_websocket, text_message};

const TERMINAL_ENDED_CODES: [&str; 3] = [
    "terminal_exited",
    "terminal_not_found",
    "terminal_not_running",
];

#[derive(Debug, Clone)]
pub struct TerminalConfig {
    pub reconnect_delays: Vec<Duration>,
    pub heartbeat_interval: Duration,
    pub heartbeat_timeout: Duration,
    pub input_queue_max_bytes: usize,
    pub connect_timeout: Duration,
    pub auto_ack: bool,
    pub since_seq: u64,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            reconnect_delays: [100, 250, 500, 1_000, 2_000]
                .into_iter()
                .map(Duration::from_millis)
                .collect(),
            heartbeat_interval: Duration::from_secs(15),
            heartbeat_timeout: Duration::from_secs(45),
            input_queue_max_bytes: 1024 * 1024,
            connect_timeout: Duration::from_secs(15),
            auto_ack: true,
            since_seq: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalConnectionState {
    Connecting,
    Attached,
    Reconnecting,
    Closed,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TerminalEvent {
    State(TerminalConnectionState),
    Message(TerminalServerMessage),
    SequenceGap { expected: u64, received: u64 },
    InvalidMessage(String),
    InputOverflow { max_bytes: usize },
    ConnectFailed(String),
}

#[derive(Debug)]
enum Command {
    Input(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Ack(u64),
    Visible(bool),
    Close,
}

#[derive(Debug, Clone)]
pub struct TerminalConnection {
    commands: mpsc::UnboundedSender<Command>,
}

impl TerminalConnection {
    pub fn spawn(
        client: BbClient,
        terminal_id: &str,
        config: TerminalConfig,
    ) -> (Self, mpsc::UnboundedReceiver<TerminalEvent>) {
        let (commands, command_rx) = mpsc::unbounded_channel();
        let (events, event_rx) = mpsc::unbounded_channel();
        let actor = TerminalActor {
            client,
            path: terminal_ws_path(terminal_id),
            next_output_seq: config.since_seq,
            config,
            commands: command_rx,
            events,
            last_resize: None,
            visible: true,
            pending: VecDeque::new(),
            pending_bytes: 0,
            reconnect_attempt: 0,
            ended: false,
            last_pong: Instant::now(),
        };
        tokio::spawn(actor.run());
        (Self { commands }, event_rx)
    }

    pub fn send_input(&self, bytes: &[u8]) -> bool {
        self.commands.send(Command::Input(bytes.to_vec())).is_ok()
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        let _ = self.commands.send(Command::Resize { cols, rows });
    }

    pub fn acknowledge_output(&self, next_seq: u64) {
        let _ = self.commands.send(Command::Ack(next_seq));
    }

    pub fn set_visible(&self, visible: bool) {
        let _ = self.commands.send(Command::Visible(visible));
    }

    pub fn close(&self) {
        let _ = self.commands.send(Command::Close);
    }

    pub fn is_running(&self) -> bool {
        !self.commands.is_closed()
    }
}

pub fn terminal_socket_url(base: &Url, since_seq: u64) -> Url {
    let mut url = base.clone();
    let query = serde_urlencoded::to_string(TerminalWebSocketQuery {
        since_seq,
        output_acks: true,
    })
    .unwrap_or_default();
    url.set_query(Some(&query));
    url
}

trait Encode {
    fn encode(&self) -> String;
}

impl Encode for TerminalClientMessage {
    fn encode(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

enum SessionEnd {
    Closed,
    Lost,
}

struct TerminalActor {
    client: BbClient,
    path: String,
    config: TerminalConfig,
    commands: mpsc::UnboundedReceiver<Command>,
    events: mpsc::UnboundedSender<TerminalEvent>,
    next_output_seq: u64,
    last_resize: Option<(u16, u16)>,
    visible: bool,
    pending: VecDeque<String>,
    pending_bytes: usize,
    reconnect_attempt: usize,
    ended: bool,
    last_pong: Instant,
}

impl TerminalActor {
    fn emit(&self, event: TerminalEvent) {
        let _ = self.events.send(event);
    }

    fn enqueue_input(&mut self, bytes: &[u8]) {
        if self.pending_bytes + bytes.len() > self.config.input_queue_max_bytes {
            self.emit(TerminalEvent::InputOverflow {
                max_bytes: self.config.input_queue_max_bytes,
            });
            return;
        }
        for chunk in bytes.chunks(TERMINAL_DATA_MAX_BYTES) {
            self.pending
                .push_back(TerminalClientMessage::input(chunk).encode());
        }
        self.pending_bytes += bytes.len();
    }

    fn apply_offline(&mut self, command: Option<Command>) -> bool {
        match command {
            None | Some(Command::Close) => return false,
            Some(Command::Input(bytes)) => self.enqueue_input(&bytes),
            Some(Command::Resize { cols, rows }) => self.last_resize = Some((cols, rows)),
            Some(Command::Visible(visible)) => self.visible = visible,
            Some(Command::Ack(_)) => {}
        }
        true
    }

    async fn run(mut self) {
        self.emit(TerminalEvent::State(TerminalConnectionState::Connecting));
        loop {
            let base = self.client.endpoint().websocket_url(&self.path);
            let url = terminal_socket_url(&base, self.next_output_seq);
            let client = self.client.clone();
            let timeout = self.config.connect_timeout;
            let connect = async move { open_websocket(&client, &url, timeout).await };
            tokio::pin!(connect);
            let opened = loop {
                tokio::select! {
                    result = &mut connect => break Some(result),
                    command = self.commands.recv() => {
                        if !self.apply_offline(command) {
                            break None;
                        }
                    }
                }
            };
            let end = match opened {
                None => SessionEnd::Closed,
                Some(Ok(stream)) => self.run_session(stream).await,
                Some(Err(WsOpenError::Rejected { status, .. }))
                    if status == 404 || status == 410 =>
                {
                    self.emit(TerminalEvent::ConnectFailed(format!(
                        "terminal socket rejected with HTTP {status}"
                    )));
                    SessionEnd::Closed
                }
                Some(Err(error)) => {
                    self.emit(TerminalEvent::ConnectFailed(error.describe()));
                    SessionEnd::Lost
                }
            };
            if matches!(end, SessionEnd::Closed) || self.ended {
                self.emit(TerminalEvent::State(TerminalConnectionState::Closed));
                return;
            }
            if !self.wait_reconnect().await {
                self.emit(TerminalEvent::State(TerminalConnectionState::Closed));
                return;
            }
        }
    }

    async fn wait_reconnect(&mut self) -> bool {
        self.emit(TerminalEvent::State(TerminalConnectionState::Reconnecting));
        let delays = &self.config.reconnect_delays;
        let delay = delays
            .get(self.reconnect_attempt.min(delays.len().saturating_sub(1)))
            .copied()
            .unwrap_or_default();
        self.reconnect_attempt += 1;
        let deadline = Instant::now() + delay;
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => return true,
                command = self.commands.recv() => {
                    if !self.apply_offline(command) {
                        return false;
                    }
                }
            }
        }
    }

    fn handle_server_text(&mut self, text: &str) -> Option<u64> {
        let message = match serde_json::from_str::<TerminalServerMessage>(text) {
            Ok(TerminalServerMessage::Unknown(_)) | Err(_) => {
                self.emit(TerminalEvent::InvalidMessage(
                    text.chars().take(200).collect(),
                ));
                return None;
            }
            Ok(message) => message,
        };
        let mut ack = None;
        match &message {
            TerminalServerMessage::Attached {
                replay_start_seq, ..
            } => {
                self.reconnect_attempt = 0;
                self.emit(TerminalEvent::State(TerminalConnectionState::Attached));
                if *replay_start_seq > self.next_output_seq {
                    self.emit(TerminalEvent::SequenceGap {
                        expected: self.next_output_seq,
                        received: *replay_start_seq,
                    });
                    self.next_output_seq = *replay_start_seq;
                }
            }
            TerminalServerMessage::Output { chunk } => {
                if chunk.seq < self.next_output_seq {
                    return None;
                }
                if chunk.seq > self.next_output_seq {
                    self.emit(TerminalEvent::SequenceGap {
                        expected: self.next_output_seq,
                        received: chunk.seq,
                    });
                }
                self.next_output_seq = chunk.seq + 1;
                ack = self.config.auto_ack.then_some(self.next_output_seq);
            }
            TerminalServerMessage::Pong => self.last_pong = Instant::now(),
            TerminalServerMessage::Exited { .. } => self.ended = true,
            TerminalServerMessage::Error { code, .. }
                if TERMINAL_ENDED_CODES.contains(&code.as_str()) =>
            {
                self.ended = true;
            }
            _ => {}
        }
        self.emit(TerminalEvent::Message(message));
        ack
    }

    async fn run_session(&mut self, stream: WsStream) -> SessionEnd {
        let (mut sink, mut incoming) = stream.split();
        let mut opening = Vec::new();
        if let Some((cols, rows)) = self.last_resize {
            opening.push(TerminalClientMessage::Resize { cols, rows }.encode());
        }
        if !self.visible {
            opening.push(TerminalClientMessage::Visibility { visible: false }.encode());
        }
        opening.extend(self.pending.drain(..));
        self.pending_bytes = 0;
        for payload in opening {
            if sink.send(text_message(payload)).await.is_err() {
                return SessionEnd::Lost;
            }
        }
        self.last_pong = Instant::now();
        let mut heartbeat = tokio::time::interval_at(
            Instant::now() + self.config.heartbeat_interval,
            self.config.heartbeat_interval,
        );
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let end = loop {
            let outgoing = tokio::select! {
                command = self.commands.recv() => match command {
                    None | Some(Command::Close) => break SessionEnd::Closed,
                    Some(Command::Input(bytes)) => TerminalClientMessage::input_chunks(&bytes)
                        .into_iter()
                        .map(|message| message.encode())
                        .collect(),
                    Some(Command::Resize { cols, rows }) => {
                        if self.last_resize == Some((cols, rows)) {
                            Vec::new()
                        } else {
                            self.last_resize = Some((cols, rows));
                            vec![TerminalClientMessage::Resize { cols, rows }.encode()]
                        }
                    }
                    Some(Command::Visible(visible)) => {
                        if self.visible == visible {
                            Vec::new()
                        } else {
                            self.visible = visible;
                            vec![TerminalClientMessage::Visibility { visible }.encode()]
                        }
                    }
                    Some(Command::Ack(next_seq)) => {
                        vec![TerminalClientMessage::Ack { next_seq }.encode()]
                    }
                },
                frame = incoming.next() => match frame {
                    None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break SessionEnd::Lost,
                    Some(Ok(Message::Text(text))) => {
                        self.handle_server_text(text.as_str()).map(|next_seq| TerminalClientMessage::Ack { next_seq }.encode())
                            .into_iter()
                            .collect()
                    }
                    Some(Ok(_)) => Vec::new(),
                },
                _ = heartbeat.tick() => {
                    if self.last_pong.elapsed() > self.config.heartbeat_timeout {
                        break SessionEnd::Lost;
                    }
                    vec![TerminalClientMessage::Ping.encode()]
                }
            };
            let mut failed = false;
            for payload in outgoing {
                if sink.send(text_message(payload)).await.is_err() {
                    failed = true;
                    break;
                }
            }
            if failed {
                break SessionEnd::Lost;
            }
            if self.ended {
                break SessionEnd::Closed;
            }
        };
        let _ = sink.send(Message::Close(None)).await;
        let _ = sink.close().await;
        end
    }
}
