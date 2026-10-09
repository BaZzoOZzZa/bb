use std::collections::BTreeMap;
use std::time::Duration;

use bb_wire::JsonValue;
use bb_wire::realtime::{ChangedMessage, ClientMessage, ServerMessage, SubscriptionTarget};
use bb_wire::routes::REALTIME_WS_PATH;
use futures::{SinkExt, StreamExt};
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;

use crate::endpoint::ServerEndpoint;
use crate::http::BbClient;
use crate::ws::{WsOpenError, WsStream, open_websocket, text_message};

#[derive(Debug, Clone)]
pub struct RealtimeConfig {
    pub initial_reconnect_delay: Duration,
    pub max_reconnect_delay: Duration,
    pub reconnect_multiplier: f64,
    pub ping_interval: Duration,
    pub pong_timeout: Duration,
    pub connect_timeout: Duration,
    pub event_capacity: usize,
}

impl Default for RealtimeConfig {
    fn default() -> Self {
        Self {
            initial_reconnect_delay: Duration::from_secs(1),
            max_reconnect_delay: Duration::from_secs(30),
            reconnect_multiplier: 1.5,
            ping_interval: Duration::from_secs(20),
            pong_timeout: Duration::from_secs(10),
            connect_timeout: Duration::from_secs(15),
            event_capacity: 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RealtimeEvent {
    Connecting {
        reconnect: bool,
    },
    Connected,
    Resumed,
    Disconnected {
        retry_in: Option<Duration>,
        error: Option<String>,
    },
    Changed(ChangedMessage),
    PluginSignal {
        plugin_id: String,
        channel: String,
        payload: JsonValue,
    },
    Server(ServerMessage),
}

#[derive(Debug)]
enum Command {
    Acquire(SubscriptionTarget),
    Release(SubscriptionTarget),
    Reconnect,
    Shutdown,
}

#[derive(Debug, Clone)]
pub struct RealtimeClient {
    commands: mpsc::UnboundedSender<Command>,
    events: broadcast::Sender<RealtimeEvent>,
}

#[derive(Debug)]
pub struct Subscription {
    target: SubscriptionTarget,
    commands: mpsc::UnboundedSender<Command>,
}

impl Subscription {
    pub fn target(&self) -> &SubscriptionTarget {
        &self.target
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Release(self.target.clone()));
    }
}

impl RealtimeClient {
    pub fn spawn(client: BbClient) -> Self {
        Self::spawn_with(client, RealtimeConfig::default())
    }

    pub fn spawn_with(client: BbClient, config: RealtimeConfig) -> Self {
        let (commands, command_rx) = mpsc::unbounded_channel();
        let (events, _) = broadcast::channel(config.event_capacity.max(16));
        let actor = Actor {
            endpoint_rx: client.watch_endpoint(),
            client,
            delay: config.initial_reconnect_delay,
            config,
            commands: command_rx,
            events: events.clone(),
            targets: BTreeMap::new(),
            reconnecting: false,
            auth_retried: false,
        };
        tokio::spawn(actor.run());
        Self { commands, events }
    }

    pub fn events(&self) -> broadcast::Receiver<RealtimeEvent> {
        self.events.subscribe()
    }

    pub fn subscribe(&self, target: SubscriptionTarget) -> Subscription {
        let _ = self.commands.send(Command::Acquire(target.clone()));
        Subscription {
            target,
            commands: self.commands.clone(),
        }
    }

    pub fn reconnect(&self) {
        let _ = self.commands.send(Command::Reconnect);
    }

    pub fn shutdown(&self) {
        let _ = self.commands.send(Command::Shutdown);
    }
}

enum Outcome {
    Shutdown,
    Idle,
    Restart,
    Lost(String),
}

enum Applied {
    Subscribe(SubscriptionTarget),
    Unsubscribe(SubscriptionTarget),
    Nothing,
    Reconnect,
    Shutdown,
}

struct Actor {
    client: BbClient,
    config: RealtimeConfig,
    commands: mpsc::UnboundedReceiver<Command>,
    events: broadcast::Sender<RealtimeEvent>,
    endpoint_rx: watch::Receiver<ServerEndpoint>,
    targets: BTreeMap<String, (SubscriptionTarget, usize)>,
    delay: Duration,
    reconnecting: bool,
    auth_retried: bool,
}

impl Actor {
    fn emit(&self, event: RealtimeEvent) {
        let _ = self.events.send(event);
    }

    fn apply(&mut self, command: Option<Command>) -> Applied {
        match command {
            None | Some(Command::Shutdown) => Applied::Shutdown,
            Some(Command::Reconnect) => Applied::Reconnect,
            Some(Command::Acquire(target)) => {
                let entry = self
                    .targets
                    .entry(target.key())
                    .or_insert_with(|| (target.clone(), 0));
                entry.1 += 1;
                if entry.1 == 1 {
                    Applied::Subscribe(target)
                } else {
                    Applied::Nothing
                }
            }
            Some(Command::Release(target)) => {
                let key = target.key();
                match self.targets.get_mut(&key) {
                    Some(entry) if entry.1 > 1 => {
                        entry.1 -= 1;
                        Applied::Nothing
                    }
                    Some(_) => {
                        self.targets.remove(&key);
                        Applied::Unsubscribe(target)
                    }
                    None => Applied::Nothing,
                }
            }
        }
    }

    fn reset_backoff(&mut self) {
        self.delay = self.config.initial_reconnect_delay;
    }

    async fn run(mut self) {
        self.endpoint_rx.mark_unchanged();
        loop {
            if self.targets.is_empty() {
                let command = self.commands.recv().await;
                if matches!(self.apply(command), Applied::Shutdown) {
                    return;
                }
                continue;
            }
            self.emit(RealtimeEvent::Connecting {
                reconnect: self.reconnecting,
            });
            match self.connect_and_run().await {
                Outcome::Shutdown => return,
                Outcome::Idle => {
                    self.reconnecting = false;
                    self.reset_backoff();
                    self.emit(RealtimeEvent::Disconnected {
                        retry_in: None,
                        error: None,
                    });
                }
                Outcome::Restart => {
                    self.reconnecting = true;
                    self.reset_backoff();
                    self.emit(RealtimeEvent::Disconnected {
                        retry_in: Some(Duration::ZERO),
                        error: None,
                    });
                }
                Outcome::Lost(error) => {
                    self.reconnecting = true;
                    let delay = self.delay;
                    self.emit(RealtimeEvent::Disconnected {
                        retry_in: Some(delay),
                        error: Some(error),
                    });
                    match self.backoff(delay).await {
                        Outcome::Shutdown => return,
                        Outcome::Idle => {
                            self.reconnecting = false;
                            self.reset_backoff();
                            self.emit(RealtimeEvent::Disconnected {
                                retry_in: None,
                                error: None,
                            });
                        }
                        Outcome::Restart => self.reset_backoff(),
                        Outcome::Lost(_) => {
                            let next = delay.mul_f64(self.config.reconnect_multiplier);
                            self.delay = next.min(self.config.max_reconnect_delay);
                        }
                    }
                }
            }
        }
    }

    async fn backoff(&mut self, delay: Duration) -> Outcome {
        let deadline = Instant::now() + delay;
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => return Outcome::Lost(String::new()),
                command = self.commands.recv() => match self.apply(command) {
                    Applied::Shutdown => return Outcome::Shutdown,
                    Applied::Reconnect => return Outcome::Restart,
                    Applied::Unsubscribe(_) if self.targets.is_empty() => return Outcome::Idle,
                    _ => {}
                },
                changed = self.endpoint_rx.changed() => {
                    if changed.is_ok() {
                        return Outcome::Restart;
                    }
                }
            }
        }
    }

    async fn connect_and_run(&mut self) -> Outcome {
        let url = self.client.endpoint().websocket_url(REALTIME_WS_PATH);
        let client = self.client.clone();
        let timeout = self.config.connect_timeout;
        let connect = async move { open_websocket(&client, &url, timeout).await };
        tokio::pin!(connect);
        let stream = loop {
            tokio::select! {
                result = &mut connect => break result,
                command = self.commands.recv() => match self.apply(command) {
                    Applied::Shutdown => return Outcome::Shutdown,
                    Applied::Unsubscribe(_) if self.targets.is_empty() => return Outcome::Idle,
                    _ => {}
                },
            }
        };
        match stream {
            Ok(stream) => {
                self.auth_retried = false;
                self.run_session(stream).await
            }
            Err(WsOpenError::Rejected {
                retried_auth: true, ..
            }) if !self.auth_retried => {
                self.auth_retried = true;
                Outcome::Restart
            }
            Err(error) => Outcome::Lost(error.describe()),
        }
    }

    async fn run_session(&mut self, stream: WsStream) -> Outcome {
        let (mut sink, mut incoming) = stream.split();
        let subscribe_all: Vec<String> = self
            .targets
            .values()
            .map(|(target, _)| {
                ClientMessage::Subscribe {
                    target: target.clone(),
                }
                .to_json()
            })
            .collect();
        for message in subscribe_all {
            if let Err(error) = sink.send(text_message(message)).await {
                return Outcome::Lost(error.to_string());
            }
        }
        self.reset_backoff();
        let event = if self.reconnecting {
            RealtimeEvent::Resumed
        } else {
            RealtimeEvent::Connected
        };
        self.reconnecting = false;
        self.emit(event);

        let mut ping = tokio::time::interval_at(
            Instant::now() + self.config.ping_interval,
            self.config.ping_interval,
        );
        ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut pong_deadline: Option<Instant> = None;
        let outcome = loop {
            let deadline = pong_deadline;
            tokio::select! {
                command = self.commands.recv() => {
                    let message = match self.apply(command) {
                        Applied::Shutdown => break Outcome::Shutdown,
                        Applied::Reconnect => break Outcome::Restart,
                        Applied::Nothing => None,
                        Applied::Subscribe(target) => Some(ClientMessage::Subscribe { target }),
                        Applied::Unsubscribe(target) => {
                            if self.targets.is_empty() {
                                break Outcome::Idle;
                            }
                            Some(ClientMessage::Unsubscribe { target })
                        }
                    };
                    if let Some(message) = message
                        && let Err(error) = sink.send(text_message(message.to_json())).await
                    {
                        break Outcome::Lost(error.to_string());
                    }
                }
                changed = self.endpoint_rx.changed() => {
                    if changed.is_ok() {
                        break Outcome::Restart;
                    }
                }
                frame = incoming.next() => {
                    pong_deadline = None;
                    match frame {
                        None => break Outcome::Lost("realtime socket closed".into()),
                        Some(Err(error)) => break Outcome::Lost(error.to_string()),
                        Some(Ok(Message::Close(_))) => {
                            break Outcome::Lost("realtime socket closed by server".into());
                        }
                        Some(Ok(Message::Text(text))) => self.handle_text(text.as_str()),
                        Some(Ok(_)) => {}
                    }
                }
                _ = ping.tick() => {
                    if let Err(error) = sink.send(text_message(ClientMessage::Ping.to_json())).await {
                        break Outcome::Lost(error.to_string());
                    }
                    if pong_deadline.is_none() {
                        pong_deadline = Some(Instant::now() + self.config.pong_timeout);
                    }
                }
                _ = sleep_until_some(deadline), if deadline.is_some() => {
                    break Outcome::Lost("realtime pong timeout".into());
                }
            }
        };
        let _ = sink.send(Message::Close(None)).await;
        let _ = sink.close().await;
        outcome
    }

    fn handle_text(&self, text: &str) {
        match ServerMessage::parse(text) {
            ServerMessage::Changed(ChangedMessage::Unknown(value)) => {
                tracing::debug!(%value, "ignoring unknown realtime change");
            }
            ServerMessage::Changed(changed) => {
                self.emit(RealtimeEvent::Changed(changed.without_unknown_kinds()));
            }
            ServerMessage::Pong => {}
            ServerMessage::PluginSignal {
                plugin_id,
                channel,
                payload,
            } => self.emit(RealtimeEvent::PluginSignal {
                plugin_id,
                channel,
                payload,
            }),
            ServerMessage::Unknown(value) => {
                tracing::debug!(%value, "ignoring unknown realtime message");
            }
            other => self.emit(RealtimeEvent::Server(other)),
        }
    }
}

async fn sleep_until_some(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}
