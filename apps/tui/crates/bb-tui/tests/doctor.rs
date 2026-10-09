use std::sync::{Arc, Mutex};

use bb_tui::doctor::{Status, diagnose};
use bb_tui::startup::Args;
use clap::Parser;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

struct MockServer {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
}

async fn respond(mut stream: TcpStream, requests: Arc<Mutex<Vec<String>>>, ws: bool) {
    let mut peek = [0u8; 512];
    let read = stream.peek(&mut peek).await.unwrap_or(0);
    let head = String::from_utf8_lossy(&peek[..read]).to_string();
    let line = head.lines().next().unwrap_or_default().to_owned();
    requests.lock().unwrap().push(line.clone());
    if line.starts_with("GET /ws") {
        if ws {
            if let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await {
                use futures::StreamExt;
                while let Some(Ok(_)) = socket.next().await {}
            }
            return;
        }
        let _ = stream
            .write_all(b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\n\r\n")
            .await;
        return;
    }
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    while !buffer.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(count) => buffer.extend_from_slice(&chunk[..count]),
        }
    }
    let body = if line.contains("/health") {
        r#"{"ok":true}"#
    } else if line.contains("/system/version") {
        r#"{"currentVersion":"0.99.0","source":"npm","updateAvailable":true,"latestVersion":"1.0.0","isDevelopment":false,"upgradeCommand":"npm i -g @get-bb/bb"}"#
    } else {
        "{}"
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
}

async fn start(ws: bool) -> MockServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let url = format!("http://{}", listener.local_addr().expect("addr"));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let shared = requests.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(respond(stream, shared.clone(), ws));
        }
    });
    MockServer { url, requests }
}

fn args(url: &str, dir: &std::path::Path) -> Args {
    Args::try_parse_from([
        "bb-tui",
        "--server-url",
        url,
        "--config",
        dir.join("config.toml").to_str().expect("path"),
        "doctor",
    ])
    .expect("args")
}

#[tokio::test]
async fn doctor_reports_discovery_version_and_websocket_with_get_only() {
    let server = start(true).await;
    let dir = tempfile::tempdir().expect("dir");
    let report = diagnose(&args(&server.url, dir.path())).await;
    let text = report.render();
    assert!(!report.failed(), "{text}");
    assert!(text.contains("(from --server-url)"), "{text}");
    assert!(text.contains("0.99.0"), "{text}");
    assert!(
        text.contains("1.0.0 available · npm i -g @get-bb/bb"),
        "{text}"
    );
    let websocket = report
        .checks()
        .find(|check| check.label == "websocket")
        .expect("websocket check");
    assert_eq!(websocket.status, Status::Ok, "{text}");
    assert!(websocket.detail.contains("/ws"), "{text}");
    let requests = server.requests.lock().unwrap().clone();
    assert!(!requests.is_empty());
    assert!(
        requests.iter().all(|line| line.starts_with("GET ")),
        "{requests:?}"
    );
}

#[tokio::test]
async fn doctor_fails_when_the_websocket_is_refused() {
    let server = start(false).await;
    let dir = tempfile::tempdir().expect("dir");
    let report = diagnose(&args(&server.url, dir.path())).await;
    assert!(report.failed());
    let websocket = report
        .checks()
        .find(|check| check.label == "websocket")
        .expect("websocket check");
    assert_eq!(websocket.status, Status::Fail);
    assert!(websocket.detail.contains("503"), "{}", report.render());
}

#[tokio::test]
async fn doctor_fails_and_skips_the_websocket_when_the_server_is_down() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let url = format!("http://{}", listener.local_addr().expect("addr"));
    drop(listener);
    let dir = tempfile::tempdir().expect("dir");
    let report = diagnose(&args(&url, dir.path())).await;
    assert!(report.failed());
    let text = report.render();
    assert!(text.contains("FAIL GET /health"), "{text}");
    assert!(text.contains("skipped (server unreachable)"), "{text}");
}
