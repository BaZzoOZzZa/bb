mod support;

use bb_store::{
    TurnDetailsKey, deferred_timeline_content_item_id, resolve_deferred_timeline_content,
};
use bb_wire::timeline::{TimelineRow, TimelineWorkRow};
use serde_json::{Value, json};
use support::{command_row, parse_row, patch};

fn deferred_command(id: &str, call_id: &str) -> Value {
    patch(
        &command_row(id, 5),
        json!({"callId": call_id, "command": "cat big", "output": "", "contentDeferred": true}),
    )
}

fn reasoning_row(id: &str, reasoning_id: &str, detail: Option<&str>, deferred: bool) -> Value {
    json!({
        "id": id,
        "kind": "system",
        "threadId": "thread-1",
        "turnId": "turn-1",
        "sourceSeqStart": 7,
        "sourceSeqEnd": 9,
        "startedAt": 7,
        "createdAt": 7,
        "systemKind": "operation",
        "operationKind": "reasoning",
        "reasoningId": reasoning_id,
        "title": "Thinking",
        "detail": detail,
        "status": "completed",
        "contentDeferred": deferred
    })
}

fn file_change_row(
    id: &str,
    call_id: &str,
    path: &str,
    diff: Option<&str>,
    deferred: bool,
) -> Value {
    json!({
        "id": id,
        "kind": "work",
        "workKind": "file-change",
        "threadId": "thread-1",
        "turnId": "turn-1",
        "sourceSeqStart": 11,
        "sourceSeqEnd": 12,
        "startedAt": 11,
        "createdAt": 11,
        "status": "completed",
        "callId": call_id,
        "change": {"path": path, "kind": "update", "movePath": null, "diff": diff, "diffStats": {"added": 3, "removed": 1}},
        "stdout": null,
        "stderr": null,
        "approvalStatus": null,
        "contentDeferred": deferred
    })
}

#[test]
fn identifies_deferred_items_by_call_or_reasoning_id() {
    assert_eq!(
        deferred_timeline_content_item_id(&parse_row(&deferred_command("row-1", "call-1"))),
        Some("call-1")
    );
    assert_eq!(
        deferred_timeline_content_item_id(&parse_row(&command_row("row-2", 1))),
        None
    );
    assert_eq!(
        deferred_timeline_content_item_id(&parse_row(&reasoning_row("r", "reason-1", None, true))),
        Some("reason-1")
    );
    let parent_change = patch(
        &reasoning_row("p", "reason-2", None, true),
        json!({"operationKind": "parent-change"}),
    );
    assert_eq!(
        deferred_timeline_content_item_id(&parse_row(&parent_change)),
        None
    );
}

#[test]
fn leaves_inline_rows_untouched() {
    let row = parse_row(&command_row("row-1", 1));
    assert_eq!(resolve_deferred_timeline_content(&row, &[]), Some(row));
}

#[test]
fn fills_command_output_from_loaded_rows_with_a_different_id() {
    let deferred = parse_row(&deferred_command("row-1", "call-1"));
    let loaded = parse_row(&patch(
        &command_row("detail-row", 5),
        json!({
            "callId": "call-1",
            "command": "cat big --full",
            "output": "full output",
            "outputPreview": {"experimental_fullOutputAvailability": "available", "totalChars": 11}
        }),
    ));
    let resolved = resolve_deferred_timeline_content(&deferred, &[loaded]).expect("resolved");
    let TimelineRow::Work(TimelineWorkRow::Command(command)) = resolved else {
        panic!("expected command row");
    };
    assert_eq!(command.base.id, "row-1");
    assert!(!command.content_deferred);
    assert_eq!(command.command, "cat big --full");
    assert_eq!(command.output, "full output");
    assert_eq!(
        command.output_preview.map(|preview| preview.total_chars),
        Some(11)
    );
}

#[test]
fn matches_file_changes_by_call_id_and_path() {
    let deferred = parse_row(&file_change_row("row-1", "call-9", "src/a.rs", None, true));
    let other_path = parse_row(&file_change_row(
        "x",
        "call-9",
        "src/b.rs",
        Some("diff b"),
        false,
    ));
    assert_eq!(
        resolve_deferred_timeline_content(&deferred, std::slice::from_ref(&other_path)),
        None
    );
    let same_path = parse_row(&file_change_row(
        "y",
        "call-9",
        "src/a.rs",
        Some("diff a"),
        false,
    ));
    let resolved =
        resolve_deferred_timeline_content(&deferred, &[other_path, same_path]).expect("resolved");
    let TimelineRow::Work(TimelineWorkRow::FileChange(change)) = resolved else {
        panic!("expected file change row");
    };
    assert_eq!(change.change.diff.as_deref(), Some("diff a"));
    assert!(!change.content_deferred);
}

#[test]
fn fills_reasoning_detail_by_reasoning_id() {
    let deferred = parse_row(&reasoning_row("row-1", "reason-1", None, true));
    let loaded = parse_row(&reasoning_row(
        "other",
        "reason-1",
        Some("long thoughts"),
        false,
    ));
    let resolved = resolve_deferred_timeline_content(&deferred, &[loaded]).expect("resolved");
    let TimelineRow::System(system) = resolved else {
        panic!("expected system row");
    };
    assert_eq!(system.detail.as_deref(), Some("long thoughts"));
    assert!(!system.content_deferred);
    assert_eq!(system.base.id, "row-1");
}

#[test]
fn reports_missing_content() {
    let deferred = parse_row(&deferred_command("row-1", "call-1"));
    let unrelated = parse_row(&command_row("other", 1));
    assert_eq!(
        resolve_deferred_timeline_content(&deferred, &[unrelated]),
        None
    );
}

#[test]
fn builds_detail_queries_for_deferred_rows() {
    let deferred = parse_row(&deferred_command("row-1", "call-1"));
    let key = TurnDetailsKey::for_deferred_row("thread-1", &deferred).expect("key");
    let query = key.query(None);
    assert_eq!(
        query_json(&query),
        json!({
            "deferContent": true,
            "itemId": "call-1",
            "turnId": "turn-1",
            "sourceSeqStart": 5,
            "sourceSeqEnd": 5
        })
    );
    let no_turn = parse_row(&patch(
        &deferred_command("row-2", "call-2"),
        json!({"turnId": null}),
    ));
    assert_eq!(TurnDetailsKey::for_deferred_row("thread-1", &no_turn), None);
}

fn query_json(query: &bb_wire::timeline::TurnSummaryDetailsQuery) -> Value {
    serde_json::to_value(query).expect("query json")
}
