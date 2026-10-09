mod support;

use std::collections::BTreeSet;
use std::time::Duration;

use bb_client::{BbClient, RealtimeClient, RealtimeConfig, RealtimeEvent, ServerEndpoint};
use bb_wire::realtime::{ChangedMessage, SubscriptionTarget, ThreadChangeKind};
use serde_json::json;
use support::{Frame, MockWs, next_event, wait_for_event};

fn fast_config() -> RealtimeConfig {
    RealtimeConfig {
        initial_reconnect_delay: Duration::from_millis(20),
        max_reconnect_delay: Duration::from_millis(60),
        reconnect_multiplier: 1.5,
        ping_interval: Duration::from_secs(60),
        pong_timeout: Duration::from_secs(60),
        connect_timeout: Duration::from_secs(2),
        event_capacity: 256,
    }
}

fn client_for(server: &MockWs) -> BbClient {
    BbClient::local(ServerEndpoint::parse(&server.url()).unwrap()).unwrap()
}

fn is_target_message(frame: &Frame) -> bool {
    matches!(
        frame.json["type"].as_str(),
        Some("subscribe" | "unsubscribe")
    )
}

fn summarize(frames: &[Frame]) -> BTreeSet<String> {
    frames
        .iter()
        .map(|frame| {
            format!(
                "{}:{}:{}",
                frame.json["type"].as_str().unwrap_or_default(),
                frame.json["target"]["kind"].as_str().unwrap_or_default(),
                frame.json["target"]["threadId"]
                    .as_str()
                    .unwrap_or_default()
            )
        })
        .collect()
}

#[tokio::test]
async fn subscriptions_are_ref_counted_and_changes_are_emitted() {
    let mut server = MockWs::start(true).await;
    let realtime = RealtimeClient::spawn_with(client_for(&server), fast_config());
    let mut events = realtime.events();

    let list_a = realtime.subscribe(SubscriptionTarget::ThreadList);
    let list_b = realtime.subscribe(SubscriptionTarget::ThreadList);
    let detail = realtime.subscribe(SubscriptionTarget::ThreadDetail {
        thread_id: "thr_1".into(),
    });

    assert_eq!(
        next_event(&mut events).await,
        RealtimeEvent::Connecting { reconnect: false }
    );
    wait_for_event(&mut events, |event| *event == RealtimeEvent::Connected).await;
    let opened = server.frames_matching(2, is_target_message).await;
    assert_eq!(
        summarize(&opened),
        BTreeSet::from([
            "subscribe:thread-detail:thr_1".to_owned(),
            "subscribe:thread-list:".to_owned(),
        ])
    );

    let handshake = &server.handshakes()[0];
    assert_eq!(handshake.path, "/ws");
    assert!(!handshake.headers.contains_key("origin"));
    assert_eq!(
        handshake
            .headers
            .get("x-bb-app-surface")
            .map(String::as_str),
        Some("api")
    );

    drop(list_b);
    server
        .expect_silence(Duration::from_millis(100), is_target_message)
        .await;
    drop(list_a);
    let released = server.frames_matching(1, is_target_message).await;
    assert_eq!(
        summarize(&released),
        BTreeSet::from(["unsubscribe:thread-list:".to_owned()])
    );

    server.send_latest(
        json!({
            "type": "changed",
            "entity": "thread",
            "id": "thr_1",
            "changes": ["events-appended", "brand-new-kind"],
            "metadata": {"timelineSequence": 7},
        })
        .to_string(),
    );
    server.send_latest(r#"{"type":"mystery","x":1}"#);
    server.send_latest(
        json!({"type":"plugin-signal","pluginId":"push-notifications","channel":"notify","payload":{"title":"Done"}})
            .to_string(),
    );
    let changed = wait_for_event(&mut events, |event| {
        matches!(event, RealtimeEvent::Changed(_))
    })
    .await;
    let RealtimeEvent::Changed(ChangedMessage::Thread {
        id,
        changes,
        metadata,
    }) = changed
    else {
        panic!("expected a thread change, got {changed:?}");
    };
    assert_eq!(id.as_deref(), Some("thr_1"));
    assert_eq!(changes, vec![ThreadChangeKind::EventsAppended]);
    assert_eq!(metadata.unwrap().timeline_sequence, Some(7));
    let signal = next_event(&mut events).await;
    assert!(matches!(
        signal,
        RealtimeEvent::PluginSignal { ref plugin_id, .. } if plugin_id == "push-notifications"
    ));

    drop(detail);
    let event = wait_for_event(&mut events, |event| {
        matches!(event, RealtimeEvent::Disconnected { .. })
    })
    .await;
    assert_eq!(
        event,
        RealtimeEvent::Disconnected {
            retry_in: None,
            error: None
        }
    );
    realtime.shutdown();
}

#[tokio::test]
async fn resubscribes_after_the_server_drops_the_connection() {
    let mut server = MockWs::start(true).await;
    let realtime = RealtimeClient::spawn_with(client_for(&server), fast_config());
    let mut events = realtime.events();
    let _list = realtime.subscribe(SubscriptionTarget::ThreadList);
    let _system = realtime.subscribe(SubscriptionTarget::System);
    wait_for_event(&mut events, |event| *event == RealtimeEvent::Connected).await;
    let first = server.frames_matching(2, is_target_message).await;
    assert!(first.iter().all(|frame| frame.connection == 0));

    server.drop_latest();
    let disconnected = wait_for_event(&mut events, |event| {
        matches!(event, RealtimeEvent::Disconnected { .. })
    })
    .await;
    assert!(matches!(
        disconnected,
        RealtimeEvent::Disconnected {
            retry_in: Some(delay),
            error: Some(_)
        } if delay == Duration::from_millis(20)
    ));
    assert_eq!(
        next_event(&mut events).await,
        RealtimeEvent::Connecting { reconnect: true }
    );
    wait_for_event(&mut events, |event| *event == RealtimeEvent::Resumed).await;
    let second = server.frames_matching(2, is_target_message).await;
    assert!(second.iter().all(|frame| frame.connection == 1));
    assert_eq!(summarize(&first), summarize(&second));
    realtime.shutdown();
}

#[tokio::test]
async fn reconnect_delay_grows_by_half_and_caps() {
    let server = MockWs::start(true).await;
    server.reject_next(&[503, 503, 503, 503]);
    let realtime = RealtimeClient::spawn_with(client_for(&server), fast_config());
    let mut events = realtime.events();
    let _list = realtime.subscribe(SubscriptionTarget::ProjectList);
    let mut delays = Vec::new();
    while delays.len() < 4 {
        if let RealtimeEvent::Disconnected {
            retry_in: Some(delay),
            error,
        } = next_event(&mut events).await
        {
            assert!(error.unwrap().contains("503"));
            delays.push(delay.as_millis());
        }
    }
    assert_eq!(delays, vec![20, 30, 45, 60]);
    wait_for_event(&mut events, |event| *event == RealtimeEvent::Resumed).await;
    assert_eq!(server.handshakes().len(), 5);
    realtime.shutdown();
}

#[tokio::test]
async fn pings_and_reconnects_when_pongs_stop() {
    let mut server = MockWs::start(false).await;
    let config = RealtimeConfig {
        ping_interval: Duration::from_millis(50),
        pong_timeout: Duration::from_millis(50),
        ..fast_config()
    };
    let realtime = RealtimeClient::spawn_with(client_for(&server), config);
    let mut events = realtime.events();
    let _list = realtime.subscribe(SubscriptionTarget::HostList);
    let ping = server
        .frames_matching(1, |frame| frame.json["type"] == "ping")
        .await;
    assert_eq!(ping[0].json, json!({"type": "ping"}));
    let event = wait_for_event(&mut events, |event| {
        matches!(event, RealtimeEvent::Disconnected { .. })
    })
    .await;
    assert!(matches!(
        event,
        RealtimeEvent::Disconnected { error: Some(ref error), .. } if error.contains("pong")
    ));
    wait_for_event(&mut events, |event| *event == RealtimeEvent::Resumed).await;
    realtime.shutdown();
}

#[tokio::test]
async fn endpoint_switch_moves_subscriptions_to_the_new_server() {
    let mut old = MockWs::start(true).await;
    let mut new = MockWs::start(true).await;
    let client = client_for(&old);
    let realtime = RealtimeClient::spawn_with(client.clone(), fast_config());
    let mut events = realtime.events();
    let _detail = realtime.subscribe(SubscriptionTarget::ThreadDetail {
        thread_id: "thr_9".into(),
    });
    wait_for_event(&mut events, |event| *event == RealtimeEvent::Connected).await;
    old.frames_matching(1, is_target_message).await;

    client.set_endpoint(ServerEndpoint::parse(&new.url()).unwrap());
    wait_for_event(&mut events, |event| *event == RealtimeEvent::Resumed).await;
    let moved = new.frames_matching(1, is_target_message).await;
    assert_eq!(
        summarize(&moved),
        BTreeSet::from(["subscribe:thread-detail:thr_9".to_owned()])
    );
    old.expect_silence(Duration::from_millis(50), is_target_message)
        .await;
    realtime.shutdown();
}

#[tokio::test]
async fn idle_client_does_not_connect() {
    let server = MockWs::start(true).await;
    let realtime = RealtimeClient::spawn_with(client_for(&server), fast_config());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(server.handshakes().len(), 0);
    let subscription = realtime.subscribe(SubscriptionTarget::EnvironmentList);
    let mut events = realtime.events();
    wait_for_event(&mut events, |event| *event == RealtimeEvent::Connected).await;
    assert_eq!(server.connection_count(), 1);
    drop(subscription);
    realtime.shutdown();
}

#[tokio::test]
async fn connect_profiles_send_the_desktop_session_cookie_on_upgrade() {
    use bb_client::connect::ConnectSession;
    use bb_client::{Auth, ConnectCredential};
    use support::{MockHttp, MockResponse};

    let sessions = MockHttp::start(|_| {
        MockResponse::json(
            200,
            json!({"cookie": {"domain": ".getbb.app", "name": "bb-desktop", "value": "abc", "expiresAt": 4_102_444_800_000_i64}}),
        )
    })
    .await;
    let mut server = MockWs::start(true).await;
    server.reject_next(&[401]);
    let session = ConnectSession::new(ConnectCredential {
        server_url: sessions.url(),
        handle: "bee".into(),
        credential: "bbcm_secret".into(),
    });
    let client = BbClient::new(
        ServerEndpoint::parse(&server.url()).unwrap(),
        Auth::Connect(std::sync::Arc::new(session)),
    )
    .unwrap();
    let realtime = RealtimeClient::spawn_with(client, fast_config());
    let mut events = realtime.events();
    let _list = realtime.subscribe(SubscriptionTarget::ThreadList);
    wait_for_event(&mut events, |event| *event == RealtimeEvent::Resumed).await;
    server.frames_matching(1, is_target_message).await;
    let handshakes = server.handshakes();
    assert_eq!(handshakes.len(), 2);
    for handshake in &handshakes {
        assert_eq!(
            handshake.headers.get("cookie").map(String::as_str),
            Some("bb-desktop=abc")
        );
        assert!(!handshake.headers.contains_key("x-bb-connect-machine"));
    }
    assert_eq!(sessions.requests().len(), 2);
    realtime.shutdown();
}
