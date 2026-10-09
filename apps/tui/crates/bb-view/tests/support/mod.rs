#![allow(dead_code)]

use bb_view::view::{SummaryKind, ViewRow, ViewWorkRow, WorkSummaryRow};
use bb_wire::timeline::{ActivityIntent, TimelineRow, TimelineWorkRow};
use serde_json::{Value, json};

pub fn with(mut value: Value, overrides: Value) -> Value {
    let (Some(target), Value::Object(source)) = (value.as_object_mut(), overrides) else {
        panic!("with() expects objects");
    };
    for (key, entry) in source {
        target.insert(key, entry);
    }
    value
}

pub fn base(id: &str) -> Value {
    json!({
        "id": id,
        "threadId": "thread-1",
        "turnId": "turn-1",
        "sourceSeqStart": 1,
        "sourceSeqEnd": 1,
        "startedAt": 1,
        "createdAt": 1,
    })
}

pub fn seq(value: Value, seq: i64) -> Value {
    with(value, json!({ "sourceSeqStart": seq, "sourceSeqEnd": seq }))
}

pub fn row(value: Value) -> TimelineRow {
    serde_json::from_value(value).expect("timeline row")
}

pub fn rows(values: Vec<Value>) -> Vec<TimelineRow> {
    values.into_iter().map(row).collect()
}

pub fn work(value: Value) -> TimelineWorkRow {
    match row(value) {
        TimelineRow::Work(work) => work,
        other => panic!("expected work row, got {other:?}"),
    }
}

pub fn view_work(value: Value) -> ViewRow {
    ViewRow::Work(Box::new(ViewWorkRow {
        row: work(value),
        child_rows: None,
        in_closed_step: false,
    }))
}

pub fn summary(kind: SummaryKind, children: Vec<Value>) -> WorkSummaryRow {
    use bb_view::view::SummaryChild;
    WorkSummaryRow {
        kind,
        base: serde_json::from_value(base("summary-1")).expect("base"),
        status: bb_wire::timeline::TimelineRowStatus::Completed,
        children: children
            .into_iter()
            .map(|value| {
                SummaryChild::Work(ViewWorkRow {
                    row: work(value),
                    child_rows: None,
                    in_closed_step: false,
                })
            })
            .collect(),
    }
}

pub fn assistant(id: &str) -> Value {
    with(
        base(id),
        json!({
            "kind": "conversation",
            "messageSeq": 1,
            "role": "assistant",
            "text": "",
            "attachments": null,
            "turnRequest": null,
        }),
    )
}

pub fn user(id: &str, text: &str) -> Value {
    with(
        base(id),
        json!({
            "kind": "conversation",
            "messageSeq": 1,
            "role": "user",
            "text": text,
            "attachments": null,
            "turnRequest": { "isGrouped": false, "kind": "message", "status": "accepted" },
        }),
    )
}

pub fn command(id: &str) -> Value {
    with(
        base(id),
        json!({
            "kind": "work",
            "workKind": "command",
            "status": "completed",
            "callId": "call-1",
            "command": "pnpm test",
            "cwd": null,
            "source": null,
            "output": "",
            "exitCode": 0,
            "completedAt": 201,
            "approvalStatus": null,
            "activityIntents": [],
        }),
    )
}

pub fn read_intent(path: &str) -> Value {
    json!({
        "type": "read",
        "command": format!("cat {path}"),
        "name": path.rsplit('/').next().unwrap_or(path),
        "path": path,
    })
}

pub fn list_intent(path: Option<&str>) -> Value {
    json!({
        "type": "list_files",
        "command": path.map_or("ls".to_owned(), |path| format!("ls {path}")),
        "path": path,
    })
}

pub fn search_intent(query: &str, path: Option<&str>) -> Value {
    json!({
        "type": "search",
        "command": format!("rg {query}{}", path.map(|p| format!(" {p}")).unwrap_or_default()),
        "query": query,
        "path": path,
    })
}

pub fn command_reading(paths: &[&str], seq_value: i64) -> Value {
    seq(
        with(
            command(&format!("read-{seq_value}")),
            json!({
                "callId": format!("read-call-{seq_value}"),
                "activityIntents": paths.iter().map(|path| read_intent(path)).collect::<Vec<_>>(),
            }),
        ),
        seq_value,
    )
}

pub fn file_change(id: &str, path: &str) -> Value {
    with(
        base(id),
        json!({
            "kind": "work",
            "workKind": "file-change",
            "status": "completed",
            "callId": "file-edit-1",
            "change": {
                "path": path,
                "kind": "update",
                "movePath": null,
                "diff": "@@ -1 +1 @@\n-before\n+after",
                "diffStats": { "added": 1, "removed": 1 },
            },
            "stdout": null,
            "stderr": null,
            "approvalStatus": null,
        }),
    )
}

pub fn tool(id: &str) -> Value {
    with(
        base(id),
        json!({
            "kind": "work",
            "workKind": "tool",
            "status": "completed",
            "callId": "tool-call-1",
            "toolName": "LookupTool",
            "toolArgs": { "query": "select:TodoWrite" },
            "output": "",
            "completedAt": 201,
            "approvalStatus": null,
        }),
    )
}

pub fn delegation(id: &str, child_rows: Vec<Value>) -> Value {
    with(
        base(id),
        json!({
            "kind": "work",
            "workKind": "delegation",
            "status": "completed",
            "callId": "delegation-call-1",
            "toolName": "spawnAgent",
            "childRef": null,
            "background": false,
            "subagentType": "reviewer",
            "description": "Review timeline grouping",
            "output": "",
            "completedAt": 501,
            "childRows": child_rows,
        }),
    )
}

pub fn thought(id: &str, seq_value: i64) -> Value {
    with(
        base(id),
        json!({
            "sourceSeqStart": seq_value,
            "sourceSeqEnd": seq_value,
            "startedAt": seq_value,
            "createdAt": seq_value + 1,
            "kind": "system",
            "systemKind": "operation",
            "operationKind": "reasoning",
            "status": "completed",
            "title": "Thought",
            "detail": "Consider the next change.",
            "completedAt": seq_value + 1,
        }),
    )
}

pub fn kinds(rows: &[ViewRow]) -> Vec<&'static str> {
    rows.iter()
        .map(|row| match row {
            ViewRow::Conversation(_) => "conversation",
            ViewRow::System(_) => "system",
            ViewRow::Work(_) => "work",
            ViewRow::Summary(summary) if summary.kind == SummaryKind::Step => "step-summary",
            ViewRow::Summary(_) => "bundle-summary",
            ViewRow::Turn(_) => "turn",
            ViewRow::Unknown(_) => "unknown",
        })
        .collect()
}

pub fn expect_summary(row: Option<&ViewRow>, kind: SummaryKind) -> &WorkSummaryRow {
    match row {
        Some(ViewRow::Summary(summary)) if summary.kind == kind => summary,
        other => panic!("expected {kind:?} summary, got {other:?}"),
    }
}

pub fn expect_work(row: Option<&ViewRow>) -> &ViewWorkRow {
    match row {
        Some(ViewRow::Work(work)) => work,
        other => panic!("expected work row, got {other:?}"),
    }
}

pub fn child_ids(summary: &WorkSummaryRow) -> Vec<String> {
    summary
        .children
        .iter()
        .map(|child| child.id().to_owned())
        .collect()
}

pub fn command_intents(summary: &WorkSummaryRow) -> Vec<ActivityIntent> {
    summary
        .work_children()
        .flat_map(|work| match &work.row {
            TimelineWorkRow::Command(command) => command.activity_intents.clone(),
            _ => Vec::new(),
        })
        .collect()
}
