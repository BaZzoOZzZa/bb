mod support;

use std::time::Duration;

use bb_client::discovery::CompatWarning;
use bb_client::{Auth, BbClient, ClientConfig, ClientError, ServerEndpoint, probe_server};
use bb_wire::plugin::PluginCliRequest;
use bb_wire::thread::{DeleteThreadRequest, ThreadListQuery};
use serde_json::json;
use support::{MockHttp, MockResponse};

fn client(url: &str) -> BbClient {
    BbClient::local(ServerEndpoint::parse(url).unwrap()).unwrap()
}

#[tokio::test]
async fn sends_bb_headers_and_json_bodies() {
    let server = MockHttp::start(|request| match request.path.as_str() {
        "/api/v1/threads" => MockResponse::json(200, json!([])),
        _ => MockResponse::json(200, json!({"ok": true})),
    })
    .await;
    let client = client(&server.url());
    let threads = client
        .list_threads(&ThreadListQuery {
            project_id: Some("proj 1".into()),
            limit: Some(5),
            ..ThreadListQuery::default()
        })
        .await
        .unwrap();
    assert!(threads.is_empty());
    let deleted = client
        .delete_thread(
            "thr/1",
            &DeleteThreadRequest {
                child_threads_confirmed: true,
            },
        )
        .await
        .unwrap();
    assert!(deleted.ok);
    client.stop_thread("thr_2").await.unwrap();

    let requests = server.requests();
    let list = &requests[0];
    assert_eq!(list.method, "GET");
    assert_eq!(list.query.as_deref(), Some("projectId=proj+1&limit=5"));
    assert_eq!(list.header("x-bb-app-surface"), Some("api"));
    assert_eq!(list.header("origin"), None);
    assert_eq!(list.header("content-type"), None);

    let delete = &requests[1];
    assert_eq!(delete.method, "DELETE");
    assert_eq!(delete.path, "/api/v1/threads/thr%2F1");
    assert_eq!(delete.header("content-type"), Some("application/json"));
    assert_eq!(delete.body_json(), json!({"childThreadsConfirmed": true}));
    assert_eq!(delete.header("origin"), None);

    let stop = &requests[2];
    assert_eq!(stop.method, "POST");
    assert_eq!(stop.path, "/api/v1/threads/thr_2/stop");
    assert_eq!(stop.header("content-type"), Some("application/json"));
    assert!(stop.body.is_empty());
}

#[tokio::test]
async fn parses_error_envelopes() {
    let server = MockHttp::start(|request| match request.path.as_str() {
        "/api/v1/threads/thr_1/stop" => MockResponse::json(
            409,
            json!({
                "code": "thread_not_writable",
                "message": "Thread is archived",
                "details": {"reason": "archived", "threadStatus": "completed"},
                "retryable": false
            }),
        ),
        "/api/v1/threads/thr_2/stop" => {
            MockResponse::text(403, "bb connect: machine not authorized\n")
        }
        _ => MockResponse::json(200, json!({"unexpected": "shape", "ok": "yes"})),
    })
    .await;
    let client = client(&server.url());

    let error = client.stop_thread("thr_1").await.unwrap_err();
    assert_eq!(error.status(), Some(409));
    assert_eq!(error.code(), Some("thread_not_writable"));
    assert!(error.api_error().unwrap().lifecycle_details().is_some());
    assert_eq!(error.to_string(), "HTTP 409: Thread is archived");

    let error = client.stop_thread("thr_2").await.unwrap_err();
    assert_eq!(error.status(), Some(403));
    assert_eq!(error.code(), None);
    assert!(error.to_string().contains("machine not authorized"));

    let error = client.stop_thread("thr_3").await.unwrap_err();
    assert!(matches!(error, ClientError::Decode { .. }), "{error:?}");
}

#[tokio::test]
async fn follows_server_moved_once_and_notifies_watchers() {
    let target = MockHttp::start(|_| MockResponse::json(200, json!({"ok": true}))).await;
    let target_url = target.url();
    let old = MockHttp::start(move |_| {
        MockResponse::json(
            410,
            json!({
                "code": "server_moved",
                "message": "This bb server moved",
                "details": {"serverUrl": target_url, "toHostName": "studio", "movedAt": 1}
            }),
        )
    })
    .await;
    let client = client(&old.url());
    let mut watcher = client.watch_endpoint();
    client.compact_thread("thr_1").await.unwrap();
    assert_eq!(client.endpoint().to_string(), target.url());
    assert!(watcher.has_changed().unwrap());
    assert_eq!(watcher.borrow_and_update().to_string(), target.url());
    assert_eq!(old.requests().len(), 1);
    assert_eq!(target.requests()[0].path, "/api/v1/threads/thr_1/compact");

    let loop_server = MockHttp::start(|_| {
        MockResponse::json(
            410,
            json!({"code": "server_moved", "message": "moved", "details": {"serverUrl": "ftp://nowhere"}}),
        )
    })
    .await;
    let error = self::client(&loop_server.url())
        .compact_thread("t")
        .await
        .unwrap_err();
    assert!(
        matches!(error, ClientError::ServerMoved(ref details) if details.server_url == "ftp://nowhere")
    );
}

#[tokio::test]
async fn times_out_and_reports_unreachable_servers() {
    let slow = MockHttp::start(|_| {
        MockResponse::json(200, json!({"ok": true})).delayed(Duration::from_millis(500))
    })
    .await;
    let config = ClientConfig {
        timeout: Duration::from_millis(100),
        ..ClientConfig::default()
    };
    let client = BbClient::with_config(
        ServerEndpoint::parse(&slow.url()).unwrap(),
        Auth::None,
        config,
    )
    .unwrap();
    let error = client.health().await.unwrap_err();
    assert!(matches!(error, ClientError::Timeout(_)), "{error:?}");

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let error = self::client(&url).health().await.unwrap_err();
    assert!(
        matches!(error, ClientError::Unreachable { .. }),
        "{error:?}"
    );
    assert!(error.is_retryable());
}

#[tokio::test]
async fn plugin_rpc_and_cli_results() {
    let server = MockHttp::start(|request| match request.path.as_str() {
        "/api/v1/plugins/automations/rpc/automations_list" => {
            MockResponse::json(200, json!({"ok": true, "result": [{"id": "a1"}]}))
        }
        "/api/v1/plugins/automations/rpc/broken" => {
            MockResponse::json(200, json!({"ok": false, "error": {"code": "bad", "message": "nope"}}))
        }
        "/api/v1/plugins/automations/rpc/missing" => MockResponse::json(
            404,
            json!({"ok": false, "error": {"code": "unknown_method", "message": "plugin \"automations\" has no rpc method \"missing\""}}),
        ),
        "/api/v1/plugins/tasks/cli" => MockResponse::text(
            200,
            "\n\n{\"exitCode\":0,\"stdout\":\"done\\n\",\"stderr\":\"\"}\n",
        ),
        _ => MockResponse::json(404, json!({})),
    })
    .await;
    let client = client(&server.url());
    let result = client
        .plugin_rpc("automations", "automations_list", &json!({}))
        .await
        .unwrap();
    assert_eq!(result, json!([{"id": "a1"}]));
    let error = client
        .plugin_rpc("automations", "broken", &json!({}))
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "plugin error: nope");
    let error = client
        .plugin_rpc_without_input("automations", "missing")
        .await
        .unwrap_err();
    assert_eq!(error.status(), Some(404));
    assert!(error.to_string().contains("no rpc method"));

    let cli = client
        .run_plugin_cli(
            "tasks",
            &PluginCliRequest {
                argv: vec!["list".into()],
                ..PluginCliRequest::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(cli.stdout, "done\n");
    let requests = server.requests();
    let cli_request = requests.last().unwrap();
    assert_eq!(cli_request.body_json(), json!({"argv": ["list"]}));
    assert_eq!(cli_request.header("content-type"), Some("application/json"));
}

#[tokio::test]
async fn probe_reports_health_and_soft_version_warnings() {
    let server = MockHttp::start(|request| match request.path.as_str() {
        "/health" => MockResponse::json(200, json!({"ok": true, "launchId": "l1"})),
        "/api/v1/system/version" => MockResponse::json(
            200,
            json!({"currentVersion": "0.40.0", "source": "npm", "updateAvailable": true, "isDevelopment": false, "upgradeCommand": "npx bb-app@latest"}),
        ),
        _ => MockResponse::json(404, json!({})),
    })
    .await;
    let report = probe_server(&client(&server.url())).await.unwrap();
    assert_eq!(report.health.launch_id.as_deref(), Some("l1"));
    assert_eq!(report.version.unwrap().current_version, "0.40.0");
    assert!(matches!(
        report.warnings.as_slice(),
        [CompatWarning::ServerTooOld { .. }]
    ));

    let unhealthy = MockHttp::start(|request| match request.path.as_str() {
        "/health" => MockResponse::json(
            200,
            json!({"ok": true, "serverMove": {"moveId": "m", "state": "activating"}}),
        ),
        _ => MockResponse::json(500, json!({"code": "internal", "message": "boom"})),
    })
    .await;
    let report = probe_server(&client(&unhealthy.url())).await.unwrap();
    assert!(report.version.is_none());
    assert!(matches!(
        report.warnings.as_slice(),
        [
            CompatWarning::ServerMoving,
            CompatWarning::VersionUnavailable(_)
        ]
    ));

    let down = MockHttp::start(|_| MockResponse::json(200, json!({"ok": false}))).await;
    assert!(probe_server(&client(&down.url())).await.is_err());
}
