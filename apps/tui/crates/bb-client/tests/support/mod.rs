#![allow(dead_code)]

use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;

pub const WAIT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl RecordedRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }

    pub fn body_json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or(serde_json::Value::Null)
    }
}

#[derive(Debug, Clone)]
pub struct MockResponse {
    pub status: u16,
    pub content_type: String,
    pub body: String,
    pub delay: Duration,
}

impl MockResponse {
    pub fn json(status: u16, body: serde_json::Value) -> Self {
        Self {
            status,
            content_type: "application/json".into(),
            body: body.to_string(),
            delay: Duration::ZERO,
        }
    }

    pub fn text(status: u16, body: &str) -> Self {
        Self {
            status,
            content_type: "text/plain".into(),
            body: body.into(),
            delay: Duration::ZERO,
        }
    }

    pub fn delayed(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

type Handler = Arc<dyn Fn(&RecordedRequest) -> MockResponse + Send + Sync>;

pub struct MockHttp {
    pub addr: SocketAddr,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

impl MockHttp {
    pub async fn start(
        handler: impl Fn(&RecordedRequest) -> MockResponse + Send + Sync + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let handler: Handler = Arc::new(handler);
        let recorded = requests.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let handler = handler.clone();
                let recorded = recorded.clone();
                tokio::spawn(async move {
                    let _ = serve_http(stream, handler, recorded).await;
                });
            }
        });
        Self { addr, requests }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

fn reason(status: u16) -> &'static str {
    StatusCode::from_u16(status)
        .ok()
        .and_then(|code| code.canonical_reason())
        .unwrap_or("Unknown")
}

async fn serve_http(
    mut stream: TcpStream,
    handler: Handler,
    recorded: Arc<Mutex<Vec<RecordedRequest>>>,
) -> std::io::Result<()> {
    let mut buffer = Vec::new();
    let header_end = loop {
        let mut chunk = [0u8; 4096];
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next().unwrap_or_default().split(' ');
    let method = request_line.next().unwrap_or_default().to_owned();
    let target = request_line.next().unwrap_or_default().to_owned();
    let headers: BTreeMap<String, String> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    let length: usize = headers
        .get("content-length")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let mut body = buffer[header_end..].to_vec();
    while body.len() < length {
        let mut chunk = vec![0u8; length - body.len()];
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path.to_owned(), Some(query.to_owned())),
        None => (target.clone(), None),
    };
    let request = RecordedRequest {
        method,
        path,
        query,
        headers,
        body,
    };
    recorded.lock().unwrap().push(request.clone());
    let response = handler(&request);
    if !response.delay.is_zero() {
        tokio::time::sleep(response.delay).await;
    }
    let payload = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response.status,
        reason(response.status),
        response.content_type,
        response.body.len(),
        response.body
    );
    stream.write_all(payload.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct Handshake {
    pub path: String,
    pub query: Option<String>,
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub connection: usize,
    pub json: serde_json::Value,
}

enum Action {
    Send(String),
    Drop,
}

#[derive(Clone, Default)]
struct WsShared {
    connections: Arc<Mutex<Vec<mpsc::UnboundedSender<Action>>>>,
    handshakes: Arc<Mutex<Vec<Handshake>>>,
    rejections: Arc<Mutex<VecDeque<u16>>>,
}

pub struct MockWs {
    pub addr: SocketAddr,
    frames: mpsc::UnboundedReceiver<Frame>,
    shared: WsShared,
}

impl MockWs {
    pub async fn start(auto_pong: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (frame_tx, frames) = mpsc::unbounded_channel();
        let shared = WsShared::default();
        let state = shared.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let state = state.clone();
                let frame_tx = frame_tx.clone();
                tokio::spawn(async move {
                    serve_ws(stream, state, frame_tx, auto_pong).await;
                });
            }
        });
        Self {
            addr,
            frames,
            shared,
        }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn reject_next(&self, statuses: &[u16]) {
        self.shared
            .rejections
            .lock()
            .unwrap()
            .extend(statuses.iter().copied());
    }

    pub fn handshakes(&self) -> Vec<Handshake> {
        self.shared.handshakes.lock().unwrap().clone()
    }

    pub fn connection_count(&self) -> usize {
        self.shared.connections.lock().unwrap().len()
    }

    pub fn send(&self, connection: usize, text: impl Into<String>) {
        let connections = self.shared.connections.lock().unwrap();
        let _ = connections[connection].send(Action::Send(text.into()));
    }

    pub fn send_latest(&self, text: impl Into<String>) {
        let connections = self.shared.connections.lock().unwrap();
        let _ = connections
            .last()
            .expect("a connection")
            .send(Action::Send(text.into()));
    }

    pub fn drop_latest(&self) {
        let connections = self.shared.connections.lock().unwrap();
        let _ = connections.last().expect("a connection").send(Action::Drop);
    }

    pub async fn next_frame(&mut self) -> Frame {
        tokio::time::timeout(WAIT, self.frames.recv())
            .await
            .expect("timed out waiting for a client frame")
            .expect("frame channel closed")
    }

    pub async fn frames_matching(
        &mut self,
        count: usize,
        predicate: impl Fn(&Frame) -> bool,
    ) -> Vec<Frame> {
        let mut found = Vec::new();
        while found.len() < count {
            let frame = self.next_frame().await;
            if predicate(&frame) {
                found.push(frame);
            }
        }
        found
    }

    pub async fn expect_silence(&mut self, duration: Duration, predicate: impl Fn(&Frame) -> bool) {
        let deadline = tokio::time::Instant::now() + duration;
        loop {
            match tokio::time::timeout_at(deadline, self.frames.recv()).await {
                Err(_) => return,
                Ok(Some(frame)) if predicate(&frame) => panic!("unexpected frame {frame:?}"),
                Ok(Some(_)) => {}
                Ok(None) => return,
            }
        }
    }
}

async fn serve_ws(
    stream: TcpStream,
    state: WsShared,
    frame_tx: mpsc::UnboundedSender<Frame>,
    auto_pong: bool,
) {
    let handshakes = state.handshakes.clone();
    let rejection = state.rejections.lock().unwrap().pop_front();
    #[allow(clippy::result_large_err)]
    let callback = move |request: &Request, response: Response| {
        let uri = request.uri();
        handshakes.lock().unwrap().push(Handshake {
            path: uri.path().to_owned(),
            query: uri.query().map(str::to_owned),
            headers: request
                .headers()
                .iter()
                .map(|(name, value)| {
                    (
                        name.as_str().to_owned(),
                        value.to_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect(),
        });
        match rejection {
            Some(status) => {
                let mut error = ErrorResponse::new(Some("rejected".into()));
                *error.status_mut() = StatusCode::from_u16(status).unwrap();
                Err(error)
            }
            None => Ok(response),
        }
    };
    let Ok(ws) = tokio_tungstenite::accept_hdr_async(stream, callback).await else {
        return;
    };
    let (action_tx, mut actions) = mpsc::unbounded_channel();
    let connection = {
        let mut connections = state.connections.lock().unwrap();
        connections.push(action_tx);
        connections.len() - 1
    };
    let (mut sink, mut incoming) = ws.split();
    loop {
        tokio::select! {
            frame = incoming.next() => match frame {
                Some(Ok(Message::Text(text))) => {
                    let json: serde_json::Value =
                        serde_json::from_str(text.as_str()).unwrap_or(serde_json::Value::Null);
                    if auto_pong && json["type"] == "ping" {
                        let _ = sink.send(Message::text(r#"{"type":"pong"}"#)).await;
                    }
                    let _ = frame_tx.send(Frame { connection, json });
                }
                Some(Ok(_)) => {}
                _ => return,
            },
            action = actions.recv() => match action {
                Some(Action::Send(text)) => {
                    if sink.send(Message::text(text)).await.is_err() {
                        return;
                    }
                }
                Some(Action::Drop) | None => return,
            },
        }
    }
}

pub async fn next_event<T: Clone + std::fmt::Debug>(
    events: &mut tokio::sync::broadcast::Receiver<T>,
) -> T {
    tokio::time::timeout(WAIT, events.recv())
        .await
        .expect("timed out waiting for an event")
        .expect("event channel closed")
}

pub async fn wait_for_event<T: Clone + std::fmt::Debug>(
    events: &mut tokio::sync::broadcast::Receiver<T>,
    predicate: impl Fn(&T) -> bool,
) -> T {
    loop {
        let event = next_event(events).await;
        if predicate(&event) {
            return event;
        }
    }
}
