mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use bb_client::{BbClient, DiscoveryInput, discover_server, discovery};
use bb_store::{NoRealtime, Store, StoreConfig};
use bb_tui::api::Api;
use bb_tui::app::{Action, AppState, ServerInfo, update};
use bb_tui::time::now_ms;
use support::render_lines;

#[tokio::test]
#[ignore = "needs a running bb server; read-only"]
async fn renders_live_sidebar_read_only() {
    let found = discover_server(
        &DiscoveryInput::from_env(None),
        discovery::process_is_running,
    )
    .expect("discover");
    let client = BbClient::local(found.endpoint.clone()).expect("client");
    let overrides = client.keybinding_overrides().await.expect("config");
    let store = Store::spawn(Arc::new(client), NoRealtime, None, StoreConfig::default());
    let _interest = store.watch_sidebar();
    let mut rx = store.subscribe();
    tokio::time::timeout(Duration::from_secs(10), async {
        while rx.borrow_and_update().sidebar.data.is_none() {
            rx.changed().await.expect("store");
        }
    })
    .await
    .expect("sidebar loaded");
    let mut state = AppState::new(
        ServerInfo {
            url: found.endpoint.to_string(),
            profile: None,
        },
        Instant::now(),
        now_ms(),
    );
    update(&mut state, Action::KeymapLoaded(Ok(overrides)));
    update(&mut state, Action::Snapshot(store.snapshot()));
    assert!(!state.rows.is_empty());
    for line in render_lines(&state, 120, 30) {
        println!("{line}");
    }
}
