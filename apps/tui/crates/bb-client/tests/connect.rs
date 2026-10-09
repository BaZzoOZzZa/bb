mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bb_client::connect::{
    ConnectError, ConnectSession, connect_http_client, redeem_machine_credential,
};
use bb_client::{Auth, BbClient, ConnectCredential, ServerEndpoint};
use bb_wire::thread::ThreadListQuery;
use serde_json::json;
use support::{MockHttp, MockResponse};

#[tokio::test]
async fn redeems_machine_codes() {
    let server = MockHttp::start(|request| {
        let body = request.body_json();
        match body["code"].as_str() {
            Some("GOOD-CODE") => {
                let host = request.header("host").unwrap_or_default().to_owned();
                let port = host.rsplit(':').next().unwrap_or_default().to_owned();
                MockResponse::json(
                    200,
                    json!({"credential": "bbcm_test", "machineId": "m1", "serverUrl": format!("http://bee.localhost:{port}")}),
                )
            }
            Some("USED-CODE") => MockResponse::json(409, json!({"error": "already-used"})),
            Some("FULL-CODE") => MockResponse::json(403, json!({"error": "machine-limit"})),
            Some("ELSEWHERE") => MockResponse::json(
                200,
                json!({"credential": "bbcm_x", "machineId": "m2", "serverUrl": "https://bee.evil.example"}),
            ),
            _ => MockResponse::text(500, "<html>oops</html>"),
        }
    })
    .await;
    let apex = format!("http://localhost:{}", server.addr.port());
    let http = connect_http_client();
    let credential = redeem_machine_credential(&http, &apex, "GOOD-CODE", Some("laptop"))
        .await
        .unwrap();
    assert_eq!(credential.handle, "bee");
    assert_eq!(credential.credential, "bbcm_test");
    let request = &server.requests()[0];
    assert_eq!(request.path, "/api/connect/redeem-machine");
    assert_eq!(request.header("content-type"), Some("application/json"));
    assert_eq!(
        request.body_json(),
        json!({"code": "GOOD-CODE", "deviceName": "laptop"})
    );

    let error = |code: &'static str| {
        let http = http.clone();
        let apex = apex.clone();
        async move {
            redeem_machine_credential(&http, &apex, code, None)
                .await
                .unwrap_err()
        }
    };
    assert_eq!(error("USED-CODE").await, ConnectError::AlreadyUsed);
    assert_eq!(error("FULL-CODE").await, ConnectError::MachineLimit);
    assert!(matches!(
        error("ELSEWHERE").await,
        ConnectError::InvalidResponse(_)
    ));
    assert!(matches!(
        error("BROKEN").await,
        ConnectError::InvalidResponse(_)
    ));
}

fn session_server(rejections: usize) -> impl Fn(&support::RecordedRequest) -> MockResponse {
    let minted = Arc::new(AtomicUsize::new(0));
    let rejected = Arc::new(AtomicUsize::new(0));
    move |request| match request.path.as_str() {
        "/api/connect/desktop-session" => {
            if request.header("x-bb-connect-machine") != Some("bbcm_secret") {
                return MockResponse::text(401, "no");
            }
            let index = minted.fetch_add(1, Ordering::SeqCst);
            MockResponse::json(
                200,
                json!({"cookie": {"domain": ".getbb.app", "name": "bb-desktop", "value": format!("cookie{index}"), "expiresAt": 4_102_444_800_000_i64}}),
            )
        }
        "/api/connect/servers" => MockResponse::json(
            200,
            json!({"servers": [{"handle": "bee", "name": "Bee", "live": true}, {"handle": "wasp", "name": "Wasp", "live": false}]}),
        ),
        "/health" => {
            if request.header("cookie").is_none() {
                return MockResponse::text(200, "<html>sign in</html>");
            }
            if rejected.fetch_add(1, Ordering::SeqCst) < rejections {
                return MockResponse::text(403, "bb connect: not your server");
            }
            MockResponse::json(200, json!({"ok": true}))
        }
        path if path.starts_with("/api/v1/") => {
            if request.header("x-bb-connect-machine") == Some("bbcm_secret")
                && request.header("cookie").is_none()
            {
                MockResponse::json(200, json!([]))
            } else {
                MockResponse::text(403, "bb connect: machine not authorized")
            }
        }
        _ => MockResponse::text(404, "not found"),
    }
}

fn credential(server: &MockHttp) -> ConnectCredential {
    ConnectCredential {
        server_url: server.url(),
        handle: "bee".into(),
        credential: "bbcm_secret".into(),
    }
}

#[tokio::test]
async fn connect_auth_uses_machine_header_for_api_and_cookie_elsewhere() {
    let server = MockHttp::start(session_server(1)).await;
    let session = Arc::new(ConnectSession::new(credential(&server)));
    let client = BbClient::new(
        ServerEndpoint::parse(&server.url()).unwrap(),
        Auth::Connect(session.clone()),
    )
    .unwrap();
    client
        .list_threads(&ThreadListQuery::default())
        .await
        .unwrap();
    assert!(client.health().await.unwrap().ok);

    let requests = server.requests();
    let paths: Vec<&str> = requests
        .iter()
        .map(|request| request.path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec![
            "/api/v1/threads",
            "/api/connect/desktop-session",
            "/health",
            "/api/connect/desktop-session",
            "/health"
        ]
    );
    assert_eq!(requests[2].header("cookie"), Some("bb-desktop=cookie0"));
    assert_eq!(requests[4].header("cookie"), Some("bb-desktop=cookie1"));
    assert!(session.renew_in().await.unwrap().as_secs() > 60);

    let servers = session.list_servers().await.unwrap();
    assert_eq!(servers.self_handle, "bee");
    assert_eq!(
        servers
            .servers
            .iter()
            .map(|server| (server.handle.as_str(), server.live))
            .collect::<Vec<_>>(),
        vec![("bee", true), ("wasp", false)]
    );
}

#[tokio::test]
async fn connect_session_reports_revoked_credentials() {
    let server = MockHttp::start(session_server(0)).await;
    let session = ConnectSession::new(ConnectCredential {
        credential: "bbcm_revoked".into(),
        ..credential(&server)
    });
    let error = session.cookie_header().await.unwrap_err();
    assert!(error.requires_pairing(), "{error:?}");
}
