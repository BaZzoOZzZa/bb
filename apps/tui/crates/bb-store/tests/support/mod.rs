#![allow(dead_code)]

use std::sync::Arc;

use bb_store::timeline_merge::{LatestTimeline, LoadedTimelineState, RowRef, row_id};
use bb_wire::timeline::{ThreadTimelineResponse, TimelineRow};
use serde_json::{Value, json};

pub fn patch(base: &Value, changes: Value) -> Value {
    let mut merged = base.clone();
    if let (Some(target), Value::Object(changes)) = (merged.as_object_mut(), changes) {
        for (key, value) in changes {
            target.insert(key, value);
        }
    }
    merged
}

pub fn parse_row(value: &Value) -> TimelineRow {
    let row: TimelineRow = serde_json::from_value(value.clone()).expect("timeline row");
    assert!(!row.is_unknown(), "fixture row parsed as unknown: {value}");
    row
}

pub fn row(value: &Value) -> RowRef {
    Arc::new(parse_row(value))
}

pub fn cursor(id: &str, sequence: i64) -> Value {
    json!({ "anchorSeq": sequence, "anchorId": id })
}

pub fn user_row(id: &str, sequence: i64) -> Value {
    user_row_with(id, sequence, sequence, "accepted")
}

pub fn user_row_with(id: &str, sequence: i64, end_sequence: i64, status: &str) -> Value {
    json!({
        "id": id,
        "threadId": "thread-1",
        "turnId": "turn-1",
        "sourceSeqStart": sequence,
        "sourceSeqEnd": end_sequence,
        "messageSeq": end_sequence,
        "startedAt": sequence,
        "createdAt": sequence,
        "kind": "conversation",
        "role": "user",
        "initiator": "user",
        "senderThreadId": null,
        "systemMessageKind": "unlabeled",
        "systemMessageSubject": null,
        "text": id,
        "mentions": [],
        "attachments": null,
        "turnRequest": { "isGrouped": false, "kind": "message", "status": status }
    })
}

pub fn command_row(id: &str, sequence: i64) -> Value {
    command_row_until(id, sequence, sequence)
}

pub fn command_row_until(id: &str, sequence: i64, end_sequence: i64) -> Value {
    json!({
        "id": id,
        "threadId": "thread-1",
        "turnId": "turn-1",
        "sourceSeqStart": sequence,
        "sourceSeqEnd": end_sequence,
        "startedAt": sequence,
        "createdAt": sequence,
        "kind": "work",
        "workKind": "command",
        "status": "completed",
        "callId": id,
        "command": "pnpm test",
        "cwd": null,
        "source": null,
        "output": "",
        "exitCode": 0,
        "completedAt": sequence,
        "approvalStatus": null,
        "activityIntents": []
    })
}

pub fn turn_summary_row(
    id: &str,
    sequence: i64,
    end_sequence: i64,
    children: Option<Vec<Value>>,
) -> Value {
    json!({
        "id": id,
        "threadId": "thread-1",
        "turnId": "turn-1",
        "sourceSeqStart": sequence,
        "sourceSeqEnd": end_sequence,
        "startedAt": sequence,
        "createdAt": sequence,
        "kind": "turn",
        "status": "completed",
        "summaryCount": 1,
        "completedAt": sequence,
        "children": children
    })
}

pub fn system_row(id: &str, sequence: i64, title: &str) -> Value {
    json!({
        "id": id,
        "kind": "system",
        "threadId": "thr_x",
        "turnId": null,
        "sourceSeqStart": sequence,
        "sourceSeqEnd": sequence,
        "startedAt": 0,
        "createdAt": 0,
        "systemKind": "debug",
        "title": title,
        "detail": null,
        "status": null
    })
}

fn source_seq_end(value: &Value) -> i64 {
    value["sourceSeqEnd"].as_i64().unwrap_or(0)
}

pub fn timeline_response_json(
    rows: &[Value],
    older_cursor: Option<Value>,
    max_seq: Option<i64>,
) -> Value {
    let max_seq = max_seq.unwrap_or_else(|| {
        rows.iter()
            .map(source_seq_end)
            .chain(
                older_cursor
                    .as_ref()
                    .map(|cursor| cursor["anchorSeq"].as_i64().unwrap_or(0)),
            )
            .max()
            .unwrap_or(0)
    });
    json!({
        "rows": rows,
        "contextBoundarySeq": null,
        "completedTurnDisplay": "collapse",
        "activePromptMode": null,
        "activeThinking": null,
        "activeWorkflows": [],
        "activeBackgroundCommands": [],
        "pendingTodos": null,
        "goal": null,
        "modelFallback": null,
        "maxSeq": max_seq,
        "timelinePage": {
            "kind": "latest",
            "segmentLimit": 20,
            "returnedSegmentCount": if rows.is_empty() { 0 } else { 1 },
            "hasOlderRows": older_cursor.is_some(),
            "olderCursor": older_cursor
        }
    })
}

pub fn set_page(response: &mut Value, key: &str, value: Value) {
    response["timelinePage"][key] = value;
}

pub fn parse_response(value: &Value) -> ThreadTimelineResponse {
    serde_json::from_value(value.clone()).expect("timeline response")
}

pub fn latest(value: &Value) -> LatestTimeline {
    LatestTimeline::from_response(parse_response(value))
}

pub fn latest_with_rows(value: &Value, rows: Vec<RowRef>) -> LatestTimeline {
    LatestTimeline::with_rows(parse_response(value), rows)
}

pub fn loaded_state(
    rows: Vec<RowRef>,
    older_cursor: Option<Value>,
    end: Option<i64>,
) -> LoadedTimelineState {
    let end = end.unwrap_or_else(|| {
        rows.iter()
            .map(|row| row.source_seq_end().unwrap_or(0))
            .chain(
                older_cursor
                    .as_ref()
                    .map(|cursor| cursor["anchorSeq"].as_i64().unwrap_or(0)),
            )
            .max()
            .unwrap_or(0)
    });
    LoadedTimelineState {
        history_snapshot: None,
        latest_window_end_sequence: Some(end),
        older_cursor: older_cursor.map(|cursor| serde_json::from_value(cursor).expect("cursor")),
        rows,
        surface_key: "thread-1:default".into(),
    }
}

pub fn parse_cursor(value: &Value) -> bb_wire::timeline::TimelinePaginationCursor {
    serde_json::from_value(value.clone()).expect("cursor")
}

pub fn ids(rows: &[RowRef]) -> Vec<&str> {
    rows.iter().map(|row| row_id(row)).collect()
}

pub fn plain(rows: &[RowRef]) -> Vec<TimelineRow> {
    rows.iter().map(|row| (**row).clone()).collect()
}

pub fn plain_values(values: &[Value]) -> Vec<TimelineRow> {
    values.iter().map(parse_row).collect()
}
