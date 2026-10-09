use std::time::Duration;

use bb_client::{
    BbClient, DiscoveryInput, RealtimeClient, RealtimeEvent, discover_server, discovery,
    probe_server,
};
use bb_wire::realtime::SubscriptionTarget;
use bb_wire::thread::ThreadListQuery;

#[tokio::test]
#[ignore = "needs a running bb server; read-only"]
async fn reads_from_a_running_server() {
    let input = DiscoveryInput::from_env(None);
    let found = discover_server(&input, discovery::process_is_running).unwrap();
    println!("server {} via {}", found.endpoint, found.source.label());
    let client = BbClient::local(found.endpoint).unwrap();
    let report = probe_server(&client).await.unwrap();
    for warning in &report.warnings {
        println!("warning: {warning}");
    }
    let bootstrap = client.sidebar_bootstrap().await.unwrap();
    println!("{} projects", bootstrap.projects.len());
    let threads = client
        .list_threads(&ThreadListQuery {
            limit: Some(5),
            ..ThreadListQuery::default()
        })
        .await
        .unwrap();
    if let Some(entry) = threads.first() {
        let id = &entry.thread.id;
        client
            .thread_timeline(id, &bb_wire::timeline::ThreadTimelineQuery::latest())
            .await
            .unwrap();
        client.thread_interactions(id).await.unwrap();
        client.queued_messages(id).await.unwrap();
    }
    client.system_config().await.unwrap();
    client.list_plugins().await.unwrap();

    let realtime = RealtimeClient::spawn(client);
    let mut events = realtime.events();
    let _list = realtime.subscribe(SubscriptionTarget::ThreadList);
    let connected = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(RealtimeEvent::Connected) = events.recv().await {
                return;
            }
        }
    })
    .await;
    assert!(connected.is_ok(), "realtime did not connect");
    realtime.shutdown();
}
