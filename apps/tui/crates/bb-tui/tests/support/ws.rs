use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use futures::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

pub const WAIT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub connection: usize,
    pub json: serde_json::Value,
}

impl Frame {
    pub fn kind(&self) -> &str {
        self.json["type"].as_str().unwrap_or_default()
    }

    pub fn input(&self) -> Option<Vec<u8>> {
        (self.kind() == "input")
            .then(|| STANDARD.decode(self.json["dataBase64"].as_str()?).ok())
            .flatten()
    }
}

enum Command {
    Send(String),
    Drop,
}

#[derive(Clone, Default)]
struct Shared {
    connections: Arc<Mutex<Vec<mpsc::UnboundedSender<Command>>>>,
    queries: Arc<Mutex<Vec<String>>>,
}

pub struct MockTerminalServer {
    pub url: String,
    frames: mpsc::UnboundedReceiver<Frame>,
    shared: Shared,
}

impl MockTerminalServer {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (frame_tx, frames) = mpsc::unbounded_channel();
        let shared = Shared::default();
        let state = shared.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let state = state.clone();
                let frame_tx = frame_tx.clone();
                tokio::spawn(serve(stream, state, frame_tx));
            }
        });
        Self {
            url,
            frames,
            shared,
        }
    }

    pub fn queries(&self) -> Vec<String> {
        self.shared.queries.lock().unwrap().clone()
    }

    pub fn connection_count(&self) -> usize {
        self.shared.connections.lock().unwrap().len()
    }

    pub fn send(&self, text: impl Into<String>) {
        let connections = self.shared.connections.lock().unwrap();
        let _ = connections
            .last()
            .expect("a connection")
            .send(Command::Send(text.into()));
    }

    pub fn drop_latest(&self) {
        let connections = self.shared.connections.lock().unwrap();
        let _ = connections
            .last()
            .expect("a connection")
            .send(Command::Drop);
    }

    pub async fn next_frame(&mut self) -> Frame {
        tokio::time::timeout(WAIT, self.frames.recv())
            .await
            .expect("timed out waiting for a client frame")
            .expect("frame channel closed")
    }

    pub async fn frame_where(&mut self, predicate: impl Fn(&Frame) -> bool) -> Frame {
        loop {
            let frame = self.next_frame().await;
            if predicate(&frame) {
                return frame;
            }
        }
    }

    pub async fn input_until(&mut self, expected: &[u8]) -> Vec<u8> {
        let mut received = Vec::new();
        while !received.ends_with(expected) {
            if let Some(bytes) = self.next_frame().await.input() {
                received.extend(bytes);
            }
        }
        received
    }
}

async fn serve(stream: TcpStream, state: Shared, frame_tx: mpsc::UnboundedSender<Frame>) {
    let queries = state.queries.clone();
    #[allow(clippy::result_large_err)]
    let callback = move |request: &Request, response: Response| {
        queries
            .lock()
            .unwrap()
            .push(request.uri().query().unwrap_or_default().to_owned());
        Ok(response)
    };
    let Ok(ws) = tokio_tungstenite::accept_hdr_async(stream, callback).await else {
        return;
    };
    let (command_tx, mut commands) = mpsc::unbounded_channel();
    let connection = {
        let mut connections = state.connections.lock().unwrap();
        connections.push(command_tx);
        connections.len() - 1
    };
    let (mut sink, mut incoming) = ws.split();
    loop {
        tokio::select! {
            frame = incoming.next() => match frame {
                Some(Ok(Message::Text(text))) => {
                    let json: serde_json::Value =
                        serde_json::from_str(text.as_str()).unwrap_or(serde_json::Value::Null);
                    if json["type"] == "ping" {
                        let _ = sink.send(Message::text(r#"{"type":"pong"}"#)).await;
                    }
                    let _ = frame_tx.send(Frame { connection, json });
                }
                Some(Ok(_)) => {}
                _ => return,
            },
            command = commands.recv() => match command {
                Some(Command::Send(text)) => {
                    if sink.send(Message::text(text)).await.is_err() {
                        return;
                    }
                }
                Some(Command::Drop) | None => return,
            },
        }
    }
}

pub fn attached(id: &str, replay_start_seq: u64, cols: u16, rows: u16) -> String {
    serde_json::json!({
        "type": "attached",
        "session": {"id": id, "threadId": "thr_wait", "hostId": "host_1", "status": "running", "cols": cols, "rows": rows, "title": format!("shell {id}")},
        "replayStartSeq": replay_start_seq,
        "nextSeq": replay_start_seq
    })
    .to_string()
}

pub fn output(seq: u64, bytes: &[u8]) -> String {
    serde_json::json!({"type": "output", "chunk": {"seq": seq, "dataBase64": STANDARD.encode(bytes)}})
        .to_string()
}

pub fn session_updated(id: &str, cols: u16, rows: u16) -> String {
    serde_json::json!({
        "type": "session-updated",
        "session": {"id": id, "threadId": "thr_wait", "hostId": "host_1", "status": "running", "cols": cols, "rows": rows, "title": format!("shell {id}")}
    })
    .to_string()
}
