mod support;

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use bb_client::terminal::terminal_socket_url;
use bb_client::{
    BbClient, ServerEndpoint, TerminalConfig, TerminalConnection, TerminalConnectionState,
    TerminalEvent,
};
use bb_wire::terminal::TerminalServerMessage;
use serde_json::json;
use support::{MockWs, WAIT};
use tokio::sync::mpsc::UnboundedReceiver;

fn fast_config() -> TerminalConfig {
    TerminalConfig {
        reconnect_delays: vec![Duration::from_millis(10), Duration::from_millis(20)],
        connect_timeout: Duration::from_secs(2),
        ..TerminalConfig::default()
    }
}

async fn next(events: &mut UnboundedReceiver<TerminalEvent>) -> TerminalEvent {
    tokio::time::timeout(WAIT, events.recv())
        .await
        .expect("timed out waiting for a terminal event")
        .expect("terminal events closed")
}

async fn wait_for(
    events: &mut UnboundedReceiver<TerminalEvent>,
    predicate: impl Fn(&TerminalEvent) -> bool,
) -> TerminalEvent {
    loop {
        let event = next(events).await;
        if predicate(&event) {
            return event;
        }
    }
}

fn attached(replay_start_seq: u64) -> String {
    json!({
        "type": "attached",
        "session": {"id": "term_1", "hostId": "host_1", "status": "running", "cols": 80, "rows": 24},
        "replayStartSeq": replay_start_seq,
        "nextSeq": replay_start_seq
    })
    .to_string()
}

fn output(seq: u64, text: &str) -> String {
    json!({"type": "output", "chunk": {"seq": seq, "dataBase64": STANDARD.encode(text)}})
        .to_string()
}

#[test]
fn socket_url_requests_replay_and_acks() {
    let base = ServerEndpoint::parse("https://bee.getbb.app")
        .unwrap()
        .websocket_url("/ws/terminals/term_1");
    assert_eq!(
        terminal_socket_url(&base, 12).as_str(),
        "wss://bee.getbb.app/ws/terminals/term_1?sinceSeq=12&outputAcks=1"
    );
}

#[tokio::test]
async fn streams_output_acks_and_replays_after_reconnect() {
    let mut server = MockWs::start(true).await;
    let client = BbClient::local(ServerEndpoint::parse(&server.url()).unwrap()).unwrap();
    let (terminal, mut events) = TerminalConnection::spawn(client, "term 1", fast_config());
    terminal.resize(100, 30);
    assert_eq!(
        next(&mut events).await,
        TerminalEvent::State(TerminalConnectionState::Connecting)
    );
    let resize = server.next_frame().await;
    assert_eq!(
        resize.json,
        json!({"type": "resize", "cols": 100, "rows": 30})
    );
    let handshake = &server.handshakes()[0];
    assert_eq!(handshake.path, "/ws/terminals/term%201");
    assert_eq!(handshake.query.as_deref(), Some("sinceSeq=0&outputAcks=1"));

    server.send_latest(attached(0));
    server.send_latest(output(0, "hello "));
    server.send_latest(output(1, "world"));
    wait_for(&mut events, |event| {
        *event == TerminalEvent::State(TerminalConnectionState::Attached)
    })
    .await;
    let mut text = Vec::new();
    while text.len() < 2 {
        if let TerminalEvent::Message(TerminalServerMessage::Output { chunk }) =
            next(&mut events).await
        {
            text.push(String::from_utf8(chunk.decode().unwrap()).unwrap());
        }
    }
    assert_eq!(text.concat(), "hello world");
    let acks = server
        .frames_matching(2, |frame| frame.json["type"] == "ack")
        .await;
    assert_eq!(acks[1].json, json!({"type": "ack", "nextSeq": 2}));

    terminal.send_input(b"ls\r");
    let input = server
        .frames_matching(1, |frame| frame.json["type"] == "input")
        .await;
    assert_eq!(input[0].json["dataBase64"], STANDARD.encode("ls\r"));

    server.drop_latest();
    wait_for(&mut events, |event| {
        *event == TerminalEvent::State(TerminalConnectionState::Reconnecting)
    })
    .await;
    terminal.send_input(b"queued");
    let reopened = server
        .frames_matching(2, |frame| frame.connection == 1)
        .await;
    assert_eq!(reopened[0].json["type"], "resize");
    assert_eq!(reopened[1].json["dataBase64"], STANDARD.encode("queued"));
    assert_eq!(
        server.handshakes()[1].query.as_deref(),
        Some("sinceSeq=2&outputAcks=1")
    );

    server.send_latest(attached(5));
    let gap = wait_for(&mut events, |event| {
        matches!(event, TerminalEvent::SequenceGap { .. })
    })
    .await;
    assert_eq!(
        gap,
        TerminalEvent::SequenceGap {
            expected: 2,
            received: 5
        }
    );
    server.send_latest(output(4, "stale"));
    server.send_latest(r#"{"type":"surprise"}"#);
    assert!(matches!(
        wait_for(&mut events, |event| !matches!(
            event,
            TerminalEvent::State(_)
        ))
        .await,
        TerminalEvent::Message(TerminalServerMessage::Attached { .. })
    ));
    assert!(matches!(
        next(&mut events).await,
        TerminalEvent::InvalidMessage(_)
    ));

    server.send_latest(
        json!({"type": "exited", "session": {"id": "term_1", "status": "exited", "exitCode": 0}})
            .to_string(),
    );
    assert!(matches!(
        next(&mut events).await,
        TerminalEvent::Message(TerminalServerMessage::Exited { .. })
    ));
    assert_eq!(
        next(&mut events).await,
        TerminalEvent::State(TerminalConnectionState::Closed)
    );
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(server.handshakes().len(), 2);
    assert!(!terminal.is_running());
}

#[tokio::test]
async fn missing_terminal_closes_without_retrying() {
    let server = MockWs::start(true).await;
    server.reject_next(&[404]);
    let client = BbClient::local(ServerEndpoint::parse(&server.url()).unwrap()).unwrap();
    let (_terminal, mut events) = TerminalConnection::spawn(client, "gone", fast_config());
    assert!(matches!(
        wait_for(&mut events, |event| matches!(event, TerminalEvent::ConnectFailed(_))).await,
        TerminalEvent::ConnectFailed(message) if message.contains("404")
    ));
    assert_eq!(
        next(&mut events).await,
        TerminalEvent::State(TerminalConnectionState::Closed)
    );
    assert_eq!(server.handshakes().len(), 1);
}

#[tokio::test]
async fn input_queue_overflow_is_reported() {
    let server = MockWs::start(true).await;
    server.reject_next(&[503, 503, 503, 503, 503, 503]);
    let client = BbClient::local(ServerEndpoint::parse(&server.url()).unwrap()).unwrap();
    let config = TerminalConfig {
        input_queue_max_bytes: 8,
        reconnect_delays: vec![Duration::from_secs(5)],
        ..fast_config()
    };
    let (terminal, mut events) = TerminalConnection::spawn(client, "t", config);
    terminal.send_input(b"12345");
    terminal.send_input(b"67890");
    assert_eq!(
        wait_for(&mut events, |event| matches!(
            event,
            TerminalEvent::InputOverflow { .. }
        ))
        .await,
        TerminalEvent::InputOverflow { max_bytes: 8 }
    );
    terminal.close();
    assert_eq!(
        wait_for(&mut events, |event| matches!(
            event,
            TerminalEvent::State(_)
        ))
        .await,
        TerminalEvent::State(TerminalConnectionState::Closed)
    );
}
