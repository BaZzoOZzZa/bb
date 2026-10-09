mod support;

use bb_view::activity_intents::ExplorationRow;
use bb_view::title::{
    SegmentAccent, StatusDecorationStatus, SummaryStyle, TimelineTitle, TitleAction,
    TitleDecoration, TitleLink, TitleOptions, TitleTone, WorkStyle,
    build_timeline_activity_intent_titles, build_timeline_row_title,
    format_timeline_decoration_text,
};
use bb_view::view::{SummaryKind, ViewRow, ViewTurnRow};
use bb_wire::timeline::{PresentationBadgeTone, TimelineRow, TimelineRowStatus, TimelineWorkRow};
use serde_json::{Value, json};
use support::*;

const DEFAULT: TitleOptions = TitleOptions {
    summary_style: SummaryStyle::Bundle,
    work_style: WorkStyle::Default,
    is_active_latest_bundle: false,
};

const BACKGROUND_SUMMARY: TitleOptions = TitleOptions {
    summary_style: SummaryStyle::Background,
    work_style: WorkStyle::Summary,
    is_active_latest_bundle: false,
};

const BACKGROUND: TitleOptions = TitleOptions {
    summary_style: SummaryStyle::Background,
    work_style: WorkStyle::Default,
    is_active_latest_bundle: false,
};

fn title(value: Value) -> TimelineTitle {
    title_with(value, DEFAULT)
}

fn title_with(value: Value, options: TitleOptions) -> TimelineTitle {
    let view = match row(value) {
        TimelineRow::Work(work) => ViewRow::Work(Box::new(bb_view::view::ViewWorkRow {
            row: work,
            child_rows: None,
            in_closed_step: false,
        })),
        TimelineRow::System(system) => ViewRow::System(system),
        TimelineRow::Conversation(conversation) => ViewRow::Conversation(conversation),
        TimelineRow::Turn(turn) => ViewRow::Turn(Box::new(ViewTurnRow {
            row: *turn,
            children: None,
        })),
        TimelineRow::Unknown(value) => ViewRow::Unknown(value),
    };
    build_timeline_row_title(&view, options)
}

fn texts(title: &TimelineTitle) -> Vec<&str> {
    title
        .segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect()
}

fn decoration_texts(title: &TimelineTitle) -> Vec<String> {
    title
        .decorations
        .iter()
        .map(format_timeline_decoration_text)
        .collect()
}

fn intent_titles(value: Value) -> Vec<bb_view::title::ActivityIntentTitle> {
    let work = work(value);
    let exploration = ExplorationRow::from_work(&work).expect("exploration row");
    build_timeline_activity_intent_titles(exploration)
}

fn command_row() -> Value {
    with(
        command("command-1"),
        json!({ "command": "pnpm exec turbo run test --filter=@bb/app", "completedAt": 2101 }),
    )
}

fn tool_row() -> Value {
    with(tool("tool-1"), json!({ "completedAt": 2101 }))
}

fn file_read_row(path: &str) -> Value {
    with(
        base("file-read-1"),
        json!({
            "kind": "work",
            "workKind": "file-read",
            "status": "completed",
            "callId": "file-read-call-1",
            "path": path,
            "cmd": null,
            "completedAt": 2101,
        }),
    )
}

fn file_change_row(
    id: &str,
    path: &str,
    kind: &str,
    diff: &str,
    added: u64,
    removed: u64,
) -> Value {
    with(
        file_change(id, path),
        json!({
            "change": {
                "path": path,
                "kind": kind,
                "movePath": null,
                "diff": diff,
                "diffStats": { "added": added, "removed": removed },
            }
        }),
    )
}

fn deleted_file_row() -> Value {
    file_change_row(
        "file-1",
        "docs/react-perf-audit.md",
        "delete",
        "-line 1\n-line 2",
        0,
        2,
    )
}

fn created_file_row() -> Value {
    file_change_row(
        "file-created-1",
        "src/new-file.ts",
        "add",
        "first\nsecond\n",
        2,
        0,
    )
}

fn edited_file_row() -> Value {
    file_change_row(
        "file-edited-1",
        "src/existing-file.ts",
        "update",
        "-before\n+after",
        1,
        1,
    )
}

fn web_search_row() -> Value {
    with(
        base("web-search-1"),
        json!({
            "kind": "work",
            "workKind": "web-search",
            "status": "completed",
            "callId": "web-search-call-1",
            "queries": ["timeline renderer"],
            "completedAt": null,
        }),
    )
}

fn web_fetch_row() -> Value {
    with(
        base("web-fetch-1"),
        json!({
            "kind": "work",
            "workKind": "web-fetch",
            "status": "completed",
            "callId": "web-fetch-call-1",
            "url": "https://example.com/thread-view",
            "prompt": null,
            "pattern": null,
            "completedAt": null,
        }),
    )
}

fn image_view_row() -> Value {
    with(
        base("image-view-1"),
        json!({
            "kind": "work",
            "workKind": "image-view",
            "status": "completed",
            "callId": "image-view-call-1",
            "path": "/tmp/sightglass-quote-merge-check/dashboard-main.png",
            "completedAt": null,
        }),
    )
}

fn delegation_row() -> Value {
    with(
        delegation("delegation-1", Vec::new()),
        json!({
            "subagentType": "general-purpose-review-agent-with-a-long-name",
            "description": "Review correctness + plan adherence",
            "completedAt": 45001,
        }),
    )
}

fn permission_grant_row(
    lifecycle: &str,
    grant_scope: Option<&str>,
    tool_name: Option<&str>,
    reason: Option<&str>,
) -> Value {
    let status = match lifecycle {
        "pending" | "resolving" => "pending",
        "interrupted" => "interrupted",
        _ => "completed",
    };
    with(
        base(&format!("permission-grant-{lifecycle}")),
        json!({
            "kind": "work",
            "workKind": "approval",
            "status": status,
            "interactionId": "pi-permission-grant",
            "approvalKind": "permission-grant",
            "lifecycle": lifecycle,
            "grantScope": grant_scope,
            "statusReason": reason,
            "target": { "itemId": "item-permission-grant", "toolName": tool_name },
        }),
    )
}

fn file_edit_approval_row(lifecycle: &str) -> Value {
    with(
        base(&format!("file-edit-approval-{lifecycle}")),
        json!({
            "kind": "work",
            "workKind": "approval",
            "status": if lifecycle == "waiting" { "pending" } else { "interrupted" },
            "interactionId": "file-edit-call",
            "approvalKind": "file-edit",
            "lifecycle": lifecycle,
            "target": { "itemId": "file-edit-call", "toolName": null },
        }),
    )
}

fn system_row(overrides: Value) -> Value {
    with(
        with(
            base("system-1"),
            json!({
                "kind": "system",
                "systemKind": "operation",
                "operationKind": "generic",
                "title": "Thread release failed",
                "detail": null,
                "status": "error",
                "completedAt": 1,
            }),
        ),
        overrides,
    )
}

fn parent_change_row(
    parent_change: Value,
    status: &str,
    thread_name: &str,
    thread_id: Option<&str>,
) -> Value {
    let action = parent_change["action"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let verb = match action.as_str() {
        "release" => "released from",
        "transfer" => "transferred to",
        _ => "assigned to",
    };
    let parent_title = if action == "release" {
        &parent_change["previousParentThreadTitle"]
    } else {
        &parent_change["nextParentThreadTitle"]
    };
    let parent = parent_title
        .as_str()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or("parent");
    let mut value = system_row(json!({
        "id": format!("system-parent-{action}"),
        "operationKind": "parent-change",
        "parentChange": parent_change,
        "title": format!("{thread_name} {verb} {parent}"),
        "status": status,
    }));
    if let Some(thread_id) = thread_id {
        value = with(value, json!({ "threadId": thread_id }));
    }
    value
}

fn turn_row(overrides: Value) -> Value {
    with(
        with(
            base("turn-1"),
            json!({
                "kind": "turn",
                "status": "completed",
                "summaryCount": 1,
                "completedAt": 3_661_001,
                "children": null,
            }),
        ),
        overrides,
    )
}

fn summary_title(
    kind: SummaryKind,
    children: Vec<Value>,
    status: TimelineRowStatus,
    options: TitleOptions,
) -> TimelineTitle {
    let mut summary = summary(kind, children);
    summary.status = status;
    build_timeline_row_title(&ViewRow::Summary(Box::new(summary)), options)
}

fn status_decoration(
    status: StatusDecorationStatus,
    duration_ms: Option<i64>,
    emphasis: bool,
) -> TitleDecoration {
    TitleDecoration::Status {
        status,
        duration_ms,
        emphasis,
    }
}

#[test]
fn keeps_command_content_separate_from_prefix_and_duration() {
    let title = title(command_row());
    assert_eq!(
        title.plain,
        "Ran pnpm exec turbo run test --filter=@bb/app (2s)"
    );
    assert_eq!(
        texts(&title),
        ["Ran", "pnpm exec turbo run test --filter=@bb/app"]
    );
    assert!(title.segments[1].em);
    assert_eq!(
        title.decorations,
        [TitleDecoration::Duration {
            started_at: 1,
            completed_at: Some(2101),
            em: false
        }]
    );
}

#[test]
fn collapses_newlines_in_multi_line_commands() {
    let title = title(with(
        command_row(),
        json!({ "command": "node <<'EOF'\nconst x = 1;\r\nconsole.log(x);\nEOF" }),
    ));
    assert!(!title.plain.contains('\n'));
    assert!(
        title
            .segments
            .iter()
            .all(|segment| !segment.text.contains('\n'))
    );
    assert_eq!(
        title.segments[1].text,
        "node <<'EOF' const x = 1; console.log(x); EOF"
    );
}

#[test]
fn emits_live_duration_on_pending_commands() {
    let title = title(with(
        command_row(),
        json!({ "status": "pending", "exitCode": null, "completedAt": null }),
    ));
    assert_eq!(
        title.plain,
        "Running pnpm exec turbo run test --filter=@bb/app"
    );
    assert_eq!(title.segments[0].text, "Running");
    assert!(title.segments[0].shimmer);
    assert_eq!(
        title.decorations,
        [TitleDecoration::Duration {
            started_at: 1,
            completed_at: None,
            em: false
        }]
    );
}

#[test]
fn titles_generic_tools_from_name_and_arguments() {
    let title = title(with(
        tool_row(),
        json!({ "toolName": "repository_context", "toolArgs": null }),
    ));
    assert_eq!(title.plain, "Ran tool repository_context (2s)");
}

#[test]
fn renders_completed_leaves_with_summary_tone() {
    let title = title_with(command_row(), BACKGROUND_SUMMARY);
    assert_eq!(
        title.plain,
        "Ran pnpm exec turbo run test --filter=@bb/app (2s)"
    );
    assert_eq!(title.tone, TitleTone::Summary);
    assert_eq!(
        title
            .segments
            .iter()
            .find(|segment| segment.em)
            .map(|segment| segment.text.as_str()),
        Some("pnpm exec turbo run test --filter=@bb/app")
    );
}

#[test]
fn keeps_denied_titles_muted_under_summary_tone() {
    let cases = [
        (
            with(command_row(), json!({ "approvalStatus": "denied" })),
            "Permission denied: pnpm exec turbo run test --filter=@bb/app (2s)",
        ),
        (
            with(
                command_row(),
                json!({
                    "approvalStatus": "denied",
                    "command": "cat /repo/src/app.ts",
                    "activityIntents": [read_intent("/repo/src/app.ts")],
                }),
            ),
            "Permission denied: Read /repo/src/app.ts",
        ),
        (
            with(edited_file_row(), json!({ "approvalStatus": "denied" })),
            "Permission denied: src/existing-file.ts +1 -1",
        ),
        (
            with(tool_row(), json!({ "approvalStatus": "denied" })),
            "Permission denied: LookupTool { query: select:TodoWrite } (2s)",
        ),
    ];
    for (value, expected) in cases {
        let title = title_with(value, BACKGROUND_SUMMARY);
        assert_eq!(title.plain, expected);
        assert_eq!(title.segments[0].text, "Permission denied:");
        assert_eq!(title.tone, TitleTone::Summary);
    }
}

#[test]
fn renders_permission_grant_lifecycles() {
    let cases = [
        (
            "pending",
            None,
            "Waiting for permission to use Bash",
            vec!["Waiting for permission", "to use", "Bash"],
            true,
        ),
        (
            "resolving",
            None,
            "Delivering permission to use Bash",
            vec!["Delivering permission", "to use", "Bash"],
            true,
        ),
        (
            "granted",
            Some("turn"),
            "Permission granted for this turn: Bash",
            vec!["Permission granted for this turn:", "Bash"],
            false,
        ),
        (
            "granted",
            Some("session"),
            "Permission granted for this session: Bash",
            vec!["Permission granted for this session:", "Bash"],
            false,
        ),
        (
            "denied",
            None,
            "Permission denied: Bash",
            vec!["Permission denied:", "Bash"],
            false,
        ),
        (
            "interrupted",
            None,
            "Permission grant interrupted: Bash",
            vec!["Permission grant interrupted:", "Bash"],
            false,
        ),
    ];
    for (lifecycle, scope, plain, segments, shimmer) in cases {
        let title = title(permission_grant_row(lifecycle, scope, Some("Bash"), None));
        assert_eq!(title.plain, plain);
        assert_eq!(texts(&title), segments);
        assert_eq!(title.segments[0].shimmer, shimmer);
        assert!(title.segments.iter().any(|segment| segment.em));
        assert!(decoration_texts(&title).is_empty());
    }
}

#[test]
fn renders_permission_grant_interrupted_reason_and_tool_fallback() {
    let reason = "Thread stopped by user request";
    let title = title(permission_grant_row(
        "interrupted",
        None,
        Some("Bash"),
        Some(reason),
    ));
    assert_eq!(
        title.plain,
        "Permission grant interrupted: Bash (Thread stopped by user request)"
    );
    assert!(texts(&title).contains(&"(Thread stopped by user request)"));
    let fallback = bb_view::title::build_timeline_row_title(
        &view_work(permission_grant_row("pending", None, None, None)),
        DEFAULT,
    );
    assert_eq!(fallback.plain, "Waiting for permissions");
}

#[test]
fn renders_file_edit_approval_lifecycles() {
    for (lifecycle, plain, verb, subject, shimmer) in [
        (
            "waiting",
            "Waiting for approval to edit files",
            "Waiting for approval to edit",
            "files",
            true,
        ),
        (
            "denied",
            "Permission denied: file changes",
            "Permission denied:",
            "file changes",
            false,
        ),
    ] {
        let title = title(file_edit_approval_row(lifecycle));
        assert_eq!(title.plain, plain);
        assert_eq!(texts(&title), [verb, subject]);
        assert_eq!(title.segments[0].shimmer, shimmer);
        assert!(title.segments[1].em);
    }
}

#[test]
fn keeps_error_commands_with_status_metadata() {
    let title = title(with(
        command_row(),
        json!({ "status": "error", "exitCode": 1, "completedAt": 2001 }),
    ));
    assert_eq!(
        title.plain,
        "Ran pnpm exec turbo run test --filter=@bb/app (2s, error)"
    );
    assert_eq!(title.tone, TitleTone::Default);
}

fn parent_change(
    action: &str,
    prev: (Option<&str>, Option<&str>),
    next: (Option<&str>, Option<&str>),
) -> Value {
    json!({
        "action": action,
        "previousParentThreadId": prev.0,
        "previousParentThreadTitle": prev.1,
        "nextParentThreadId": next.0,
        "nextParentThreadTitle": next.1,
    })
}

#[test]
fn renders_parent_change_actions_as_thread_verb_parent() {
    let cases = [
        (
            parent_change(
                "assign",
                (None, None),
                (Some("thr_next"), Some("Frontend Parent")),
            ),
            "Worker 3 assigned to Frontend Parent",
            ["Worker 3", "assigned to", "Frontend Parent"],
            "thr_next",
        ),
        (
            parent_change(
                "release",
                (Some("thr_prev"), Some("Frontend Parent")),
                (None, None),
            ),
            "Worker 3 released from Frontend Parent",
            ["Worker 3", "released from", "Frontend Parent"],
            "thr_prev",
        ),
        (
            parent_change(
                "transfer",
                (Some("thr_prev"), Some("Frontend Parent")),
                (Some("thr_next"), Some("Backend Parent")),
            ),
            "Worker 3 transferred to Backend Parent",
            ["Worker 3", "transferred to", "Backend Parent"],
            "thr_next",
        ),
    ];
    for (change, plain, segments, parent_link) in cases {
        let title = title(parent_change_row(change, "completed", "Worker 3", None));
        assert_eq!(title.plain, plain);
        assert_eq!(texts(&title), segments);
        assert!(title.segments[0].em);
        assert_eq!(
            title.segments[0].link,
            Some(TitleLink::Thread {
                thread_id: "thread-1".into()
            })
        );
        assert_eq!(title.segments[1].link, None);
        assert_eq!(title.segments[1].accent, Some(SegmentAccent::Muted));
        assert!(title.segments[2].em);
        assert_eq!(
            title.segments[2].link,
            Some(TitleLink::Thread {
                thread_id: parent_link.into()
            })
        );
    }
}

#[test]
fn renders_parent_change_thread_unlinked_without_thread_id() {
    let title = title(parent_change_row(
        parent_change(
            "assign",
            (None, None),
            (Some("thr_next"), Some("Frontend Parent")),
        ),
        "completed",
        "Worker 3",
        Some(""),
    ));
    assert_eq!(title.plain, "Worker 3 assigned to Frontend Parent");
    assert_eq!(title.segments[0].text, "Worker 3");
    assert!(title.segments[0].em);
    assert_eq!(title.segments[0].link, None);
}

#[test]
fn preserves_parent_change_thread_names_containing_the_verb_phrase() {
    for (change, name, segments) in [
        (
            parent_change(
                "assign",
                (None, None),
                (Some("thr_next"), Some("Frontend Parent")),
            ),
            "Cleanup assigned to Sam",
            ["Cleanup assigned to Sam", "assigned to", "Frontend Parent"],
        ),
        (
            parent_change(
                "release",
                (Some("thr_prev"), Some("Frontend Parent")),
                (None, None),
            ),
            "Cleanup released from Sam",
            [
                "Cleanup released from Sam",
                "released from",
                "Frontend Parent",
            ],
        ),
        (
            parent_change(
                "transfer",
                (Some("thr_prev"), Some("Frontend Parent")),
                (Some("thr_next"), Some("Backend Parent")),
            ),
            "Cleanup transferred to Sam",
            [
                "Cleanup transferred to Sam",
                "transferred to",
                "Backend Parent",
            ],
        ),
    ] {
        let title = title(parent_change_row(change, "completed", name, None));
        assert_eq!(texts(&title), segments);
        assert_eq!(
            title.segments[0].link,
            Some(TitleLink::Thread {
                thread_id: "thread-1".into()
            })
        );
    }
}

#[test]
fn falls_back_to_unlinked_parent_or_parent_id() {
    let title_none = title(parent_change_row(
        parent_change("assign", (None, None), (None, None)),
        "completed",
        "Worker 3",
        None,
    ));
    assert_eq!(title_none.plain, "Worker 3 assigned to parent");
    assert_eq!(texts(&title_none), ["Worker 3", "assigned to", "parent"]);
    assert_eq!(title_none.segments[2].link, None);
    let title_id = title(parent_change_row(
        parent_change("assign", (None, None), (Some("thr_xyz"), None)),
        "completed",
        "Worker 3",
        None,
    ));
    assert_eq!(title_id.segments[2].text, "thr_xyz");
    assert_eq!(
        title_id.segments[2].link,
        Some(TitleLink::Thread {
            thread_id: "thr_xyz".into()
        })
    );
}

#[test]
fn renders_parent_change_status_decorations() {
    for (status, shimmer, decoration) in [
        ("pending", true, None),
        ("error", false, Some("(error)")),
        ("interrupted", false, Some("(interrupted)")),
    ] {
        let title = title(parent_change_row(
            parent_change(
                "assign",
                (None, None),
                (Some("thr_next"), Some("Frontend Parent")),
            ),
            status,
            "Worker 3",
            None,
        ));
        assert_eq!(
            texts(&title),
            ["Worker 3", "assigned to", "Frontend Parent"]
        );
        assert_eq!(title.segments[0].shimmer, shimmer);
        assert_eq!(title.tone, TitleTone::Default);
        match decoration {
            Some(text) => assert!(decoration_texts(&title).contains(&text.to_owned())),
            None => assert!(title.decorations.is_empty()),
        }
    }
}

#[test]
fn renders_failed_exploration_intents_with_the_intent_verb() {
    let title = title(with(
        file_read_row("/repo/src/app.ts"),
        json!({ "status": "error" }),
    ));
    assert_eq!(title.plain, "Read /repo/src/app.ts (error)");
    assert_eq!(title.tone, TitleTone::Default);
    assert_eq!(
        title.decorations,
        [status_decoration(
            StatusDecorationStatus::Error,
            None,
            false
        )]
    );
}

#[test]
fn renders_file_change_titles_with_diff_stats() {
    let deleted = title(deleted_file_row());
    assert_eq!(deleted.plain, "Deleted docs/react-perf-audit.md -2");
    assert_eq!(
        deleted
            .segments
            .iter()
            .find(|segment| segment.em)
            .map(|s| s.text.as_str()),
        Some("react-perf-audit.md")
    );
    assert_eq!(
        deleted.decorations,
        [TitleDecoration::DiffStats {
            added: 0,
            removed: 2
        }]
    );
    let created = title(created_file_row());
    assert_eq!(created.plain, "Created src/new-file.ts +2");
    assert_eq!(created.segments[0].text, "Created");
    assert_eq!(created.segments[1].text, "new-file.ts");
    assert_eq!(created.segments[1].accent, Some(SegmentAccent::File));
    assert_eq!(
        created.decorations,
        [TitleDecoration::DiffStats {
            added: 2,
            removed: 0
        }]
    );
}

#[test]
fn declares_open_file_diff_actions_on_file_changes_only() {
    assert_eq!(
        title(edited_file_row()).action,
        Some(TitleAction::OpenFileDiff {
            path: "src/existing-file.ts".into()
        })
    );
    assert_eq!(
        title(created_file_row()).action,
        Some(TitleAction::OpenFileDiff {
            path: "src/new-file.ts".into()
        })
    );
    let renamed = with(
        edited_file_row(),
        json!({ "change": { "path": "src/old-name.ts", "kind": "update", "movePath": "src/new-name.ts", "diff": "-before\n+after", "diffStats": { "added": 1, "removed": 1 } } }),
    );
    assert_eq!(
        title(renamed).action,
        Some(TitleAction::OpenFileDiff {
            path: "src/new-name.ts".into()
        })
    );
    assert_eq!(
        title(with(
            command("cmd-1"),
            json!({ "command": "ls", "completedAt": 1 })
        ))
        .action,
        None
    );
}

#[test]
fn renders_file_change_failure_states() {
    let failed = title(with(edited_file_row(), json!({ "status": "error" })));
    assert_eq!(failed.plain, "Failed to edit src/existing-file.ts (error)");
    let interrupted = title(with(created_file_row(), json!({ "status": "interrupted" })));
    assert_eq!(
        interrupted.plain,
        "Interrupted while creating src/new-file.ts"
    );
    let waiting = title(with(
        edited_file_row(),
        json!({ "approvalStatus": "waiting_for_approval" }),
    ));
    assert_eq!(
        waiting.plain,
        "Waiting for approval to edit src/existing-file.ts +1 -1"
    );
    let renamed = title(with(
        edited_file_row(),
        json!({ "change": { "path": "a/old.ts", "kind": "update", "movePath": "a/new.ts", "diff": "", "diffStats": { "added": 0, "removed": 0 } } }),
    ));
    assert_eq!(renamed.plain, "Renamed a/old.ts -> a/new.ts");
    assert_eq!(renamed.segments[1].text, "old.ts -> new.ts");
}

#[test]
fn emits_delegation_type_as_its_own_truncating_segment() {
    let title = title(delegation_row());
    assert_eq!(
        title.plain,
        "Ran subagent: Review correctness + plan adherence (general-purpose-review-agent-with-a-long-name) (45s)"
    );
    let segment = title
        .segments
        .iter()
        .find(|segment| segment.text == "(general-purpose-review-agent-with-a-long-name)")
        .expect("type segment");
    assert!(segment.truncate);
    assert!(!segment.em);
    assert_eq!(
        title.decorations,
        [TitleDecoration::Duration {
            started_at: 1,
            completed_at: Some(45001),
            em: false
        }]
    );
}

#[test]
fn titles_promoted_skill_delegations_by_presentation() {
    for (status, output, completed_at, expected) in [
        ("pending", "", Value::Null, "Running skill code-review"),
        (
            "completed",
            "Skill \"code-review\" completed (forked execution).",
            json!(45001),
            "Ran skill code-review (45s)",
        ),
    ] {
        let title = title(with(
            delegation_row(),
            json!({
                "status": status,
                "completedAt": completed_at,
                "toolName": "Skill",
                "subagentType": null,
                "description": null,
                "output": output,
                "presentation": {
                    "label": { "pending": "Running skill", "completed": "Ran skill" },
                    "icon": { "glyph": "Zap" },
                    "title": "code-review",
                },
            }),
        ));
        assert_eq!(title.plain, expected);
    }
}

#[test]
fn uses_lifecycle_wording_for_failed_delegations() {
    assert_eq!(
        title(with(delegation_row(), json!({ "status": "error" }))).plain,
        "Failed subagent: Review correctness + plan adherence (general-purpose-review-agent-with-a-long-name) (45s)"
    );
    assert_eq!(
        title(with(delegation_row(), json!({ "status": "interrupted" }))).plain,
        "Interrupted subagent: Review correctness + plan adherence (general-purpose-review-agent-with-a-long-name) (45s)"
    );
}

#[test]
fn flags_failed_system_operations_with_an_error_decoration() {
    let title = title(system_row(json!({})));
    assert_eq!(title.plain, "Thread release failed (error)");
    assert_eq!(title.tone, TitleTone::Default);
    assert_eq!(
        title.decorations,
        [status_decoration(StatusDecorationStatus::Error, None, true)]
    );
}

#[test]
fn renders_compaction_and_context_clear_operations() {
    let completed = title(system_row(json!({
        "id": "compaction-completed",
        "createdAt": 7001,
        "operationKind": "compaction",
        "title": "Context compacted",
        "status": "completed",
        "completedAt": 7001,
    })));
    assert_eq!(completed.plain, "Context compacted (7s)");
    let pending = title(system_row(json!({
        "operationKind": "compaction",
        "title": "Compacting context",
        "status": "pending",
        "completedAt": null,
    })));
    assert_eq!(pending.segments[0].text, "Compacting context…");
    assert!(pending.segments[0].shimmer);
    assert_eq!(
        pending.decorations,
        [TitleDecoration::Duration {
            started_at: 1,
            completed_at: None,
            em: false
        }]
    );
    let cleared = title(system_row(json!({
        "operationKind": "context-clear",
        "title": "Context cleared",
        "status": "completed",
    })));
    assert_eq!(cleared.plain, "Context cleared");
    assert!(cleared.decorations.is_empty());
}

#[test]
fn renders_reasoning_rows() {
    let thinking = title(system_row(json!({
        "operationKind": "reasoning",
        "title": "Reasoning",
        "status": "pending",
        "completedAt": null,
    })));
    assert_eq!(thinking.plain, "Thinking…");
    let thought = title(system_row(json!({
        "operationKind": "reasoning",
        "title": "Reasoning",
        "status": "completed",
        "completedAt": 5001,
    })));
    assert_eq!(thought.plain, "Thought (5s)");
}

#[test]
fn formats_turn_durations() {
    assert_eq!(title(turn_row(json!({}))).plain, "Worked for (1h 1m 1s)");
    assert_eq!(
        title(turn_row(json!({ "completedAt": 251 }))).plain,
        "Worked for (250ms)"
    );
    assert_eq!(
        title(turn_row(json!({ "completedAt": 1501 }))).plain,
        "Worked for (2s)"
    );
    let one_second = title(turn_row(
        json!({ "completedAt": 1001, "status": "pending" }),
    ));
    assert_eq!(one_second.plain, "Working");
    assert!(one_second.segments[0].shimmer);
    let live = title(turn_row(
        json!({ "completedAt": null, "status": "pending" }),
    ));
    assert_eq!(live.plain, "Working");
    assert_eq!(
        live.decorations,
        [TitleDecoration::Duration {
            started_at: 1,
            completed_at: None,
            em: true
        }]
    );
    assert_eq!(
        title(turn_row(json!({ "completedAt": null }))).plain,
        "Worked"
    );
}

#[test]
fn hides_subsecond_command_durations() {
    assert_eq!(
        title(with(command_row(), json!({ "completedAt": 251 }))).plain,
        "Ran pnpm exec turbo run test --filter=@bb/app"
    );
}

#[test]
fn renders_interrupted_titles_with_elapsed_duration() {
    let interrupted = json!({ "status": "interrupted", "completedAt": 3001 });
    for (value, expected) in [
        (
            with(
                with(command_row(), json!({ "exitCode": null })),
                interrupted.clone(),
            ),
            "Ran pnpm exec turbo run test --filter=@bb/app (3s, interrupted)",
        ),
        (
            with(tool_row(), interrupted.clone()),
            "Ran tool LookupTool { query: select:TodoWrite } (3s, interrupted)",
        ),
        (
            with(web_search_row(), interrupted.clone()),
            "Interrupted web search: timeline renderer (3s, interrupted)",
        ),
        (
            with(web_fetch_row(), interrupted.clone()),
            "Interrupted fetch: https://example.com/thread-view (3s, interrupted)",
        ),
        (
            with(image_view_row(), interrupted.clone()),
            "Interrupted image view: /tmp/sightglass-quote-merge-check/dashboard-main.png (3s, interrupted)",
        ),
    ] {
        let title = title(value);
        assert_eq!(title.plain, expected);
        assert_eq!(
            title.decorations,
            [status_decoration(
                StatusDecorationStatus::Interrupted,
                Some(3000),
                false
            )]
        );
    }
}

#[test]
fn renders_image_view_rows_with_compact_name_and_full_plain_path() {
    let completed = title(image_view_row());
    assert_eq!(
        completed.plain,
        "Viewed image: /tmp/sightglass-quote-merge-check/dashboard-main.png"
    );
    assert_eq!(completed.segments[1].text, "dashboard-main.png");
    assert_eq!(
        completed.segments[1].plain_text.as_deref(),
        Some("/tmp/sightglass-quote-merge-check/dashboard-main.png")
    );
    assert!(completed.segments[1].truncate);
    let pending = title(with(image_view_row(), json!({ "status": "pending" })));
    assert_eq!(
        pending.plain,
        "Viewing image: /tmp/sightglass-quote-merge-check/dashboard-main.png"
    );
    assert_eq!(pending.segments[0].text, "Viewing image:");
    assert!(pending.segments[0].shimmer);
}

#[test]
fn renders_step_summaries_as_bundle_or_background_titles() {
    let children = vec![web_search_row(), web_fetch_row()];
    let bundle = summary_title(
        SummaryKind::Step,
        children.clone(),
        TimelineRowStatus::Completed,
        DEFAULT,
    );
    let background = summary_title(
        SummaryKind::Step,
        children,
        TimelineRowStatus::Completed,
        BACKGROUND,
    );
    assert_eq!(bundle.plain, "Researched 1 search query, 1 web page");
    assert_eq!(texts(&bundle), ["Researched", "1 search query, 1 web page"]);
    assert_eq!(bundle.segments[0].accent, Some(SegmentAccent::Subtle));
    assert_eq!(background.plain, "Researched 1 search query, 1 web page");
    assert_eq!(background.segments.len(), 1);
    assert!(!background.segments[0].em);
    assert!(!background.segments[0].shimmer);
    assert!(background.segments[0].truncate);
    assert_eq!(background.tone, TitleTone::Summary);
}

#[test]
fn summarizes_bundled_images_and_file_changes() {
    assert_eq!(
        summary_title(
            SummaryKind::Step,
            vec![image_view_row(), image_view_row()],
            TimelineRowStatus::Completed,
            DEFAULT
        )
        .plain,
        "Viewed 2 images"
    );
    assert_eq!(
        summary_title(
            SummaryKind::Step,
            vec![created_file_row(), deleted_file_row(), edited_file_row()],
            TimelineRowStatus::Completed,
            DEFAULT
        )
        .plain,
        "Edited 3 files"
    );
}

#[test]
fn does_not_relabel_completed_summaries_as_active() {
    let title = summary_title(
        SummaryKind::Step,
        vec![web_search_row()],
        TimelineRowStatus::Completed,
        DEFAULT,
    );
    assert_eq!(title.plain, "Researched 1 search query");
    assert!(title.segments.iter().all(|segment| !segment.shimmer));
}

#[test]
fn keeps_non_success_summary_status_visible() {
    let errored = summary_title(
        SummaryKind::Step,
        vec![with(command_row(), json!({ "status": "error" }))],
        TimelineRowStatus::Error,
        BACKGROUND,
    );
    assert_eq!(errored.plain, "Ran 1 command (1 error)");
    assert_eq!(
        errored.decorations,
        [TitleDecoration::SummaryStatus {
            error_count: 1,
            interrupted_count: 0
        }]
    );
    assert_eq!(errored.tone, TitleTone::Summary);
    let interrupted = summary_title(
        SummaryKind::Step,
        vec![with(command_row(), json!({ "status": "interrupted" }))],
        TimelineRowStatus::Interrupted,
        BACKGROUND,
    );
    assert_eq!(interrupted.plain, "Ran 1 command (1 interrupted)");
}

#[test]
fn uses_active_wording_for_latest_bundle_summaries() {
    let active = TitleOptions {
        is_active_latest_bundle: true,
        ..DEFAULT
    };
    let research = summary_title(
        SummaryKind::Bundle,
        vec![with(web_search_row(), json!({ "status": "pending" }))],
        TimelineRowStatus::Pending,
        active,
    );
    assert_eq!(research.plain, "Researching 1 search query");
    assert!(research.segments.iter().any(|segment| segment.shimmer));
    let mixed = summary_title(
        SummaryKind::Bundle,
        vec![
            with(
                file_read_row("/repo/src/app.ts"),
                json!({ "status": "pending" }),
            ),
            with(command_row(), json!({ "status": "pending" })),
        ],
        TimelineRowStatus::Pending,
        active,
    );
    assert_eq!(mixed.plain, "Exploring 1 file, running 1 command");
    assert!(mixed.segments.iter().any(|segment| segment.shimmer));
}

#[test]
fn summarizes_approval_only_bundles() {
    let waiting = json!({ "approvalStatus": "waiting_for_approval" });
    assert_eq!(
        summary_title(
            SummaryKind::Bundle,
            vec![
                with(command_row(), waiting.clone()),
                with(tool_row(), waiting.clone())
            ],
            TimelineRowStatus::Pending,
            DEFAULT
        )
        .plain,
        "Waiting for approval on 1 command, 1 tool"
    );
    assert_eq!(
        summary_title(
            SummaryKind::Bundle,
            vec![
                with(command_row(), waiting),
                with(command_row(), json!({ "approvalStatus": "denied" }))
            ],
            TimelineRowStatus::Pending,
            DEFAULT
        )
        .plain,
        "Ran 2 commands"
    );
}

#[test]
fn builds_compact_exploration_intent_titles_with_read_deduping() {
    let titles = intent_titles(with(
        command_row(),
        json!({ "activityIntents": [read_intent("src/app.ts"), read_intent("src/app.ts"), search_intent("TODO", Some("src"))] }),
    ));
    let plains: Vec<&str> = titles
        .iter()
        .map(|entry| entry.title.plain.as_str())
        .collect();
    assert_eq!(plains, ["Read src/app.ts", "Searched for TODO in src"]);
    assert_eq!(titles[0].title.segments[0].text, "Read");
    assert_eq!(titles[0].title.segments[1].text, "app.ts");
    assert_eq!(titles[1].id, "command-1:activity-intent:2");
    assert!(
        titles
            .iter()
            .all(|entry| entry.title.segments.iter().all(|segment| !segment.em))
    );
}

#[test]
fn compacts_skill_read_titles() {
    let nested = "/Users/brsbl/.codex/plugins/cache/openai-bundled/browser/26.608.12217/skills/control-in-app-browser/SKILL.md";
    let nested_title = title(file_read_row(nested));
    assert_eq!(
        texts(&nested_title),
        ["Read", "control-in-app-browser/SKILL.md"]
    );
    assert_eq!(nested_title.plain, format!("Read {nested}"));
    let root = "/Users/brsbl/.codex/plugins/cache/openai-bundled/browser/26.608.12217/SKILL.md";
    let root_title = title(file_read_row(root));
    assert_eq!(texts(&root_title), ["Read", "browser/SKILL.md"]);
    assert_eq!(root_title.plain, format!("Read {root}"));
}

#[test]
fn uses_active_wording_for_pending_intent_titles() {
    let titles = intent_titles(with(
        command_row(),
        json!({ "status": "pending", "exitCode": null, "activityIntents": [read_intent("src/app.ts"), search_intent("TODO", Some("src"))] }),
    ));
    let plains: Vec<&str> = titles
        .iter()
        .map(|entry| entry.title.plain.as_str())
        .collect();
    assert_eq!(plains, ["Reading src/app.ts", "Searching for TODO in src"]);
    assert!(titles[0].title.segments[0].shimmer);
    assert_eq!(titles[0].title.segments[1].text, "app.ts");
}

#[test]
fn appends_failure_decorations_to_intent_titles() {
    let errored = intent_titles(with(
        command_row(),
        json!({ "status": "error", "exitCode": 1, "activityIntents": [read_intent("src/app.ts")] }),
    ));
    assert_eq!(errored.len(), 1);
    assert_eq!(errored[0].title.plain, "Read src/app.ts (error)");
    assert_eq!(
        errored[0].title.decorations,
        [status_decoration(
            StatusDecorationStatus::Error,
            None,
            false
        )]
    );
    let interrupted = intent_titles(with(
        command_row(),
        json!({ "status": "interrupted", "exitCode": null, "activityIntents": [search_intent("TODO", Some("src"))] }),
    ));
    assert_eq!(
        interrupted[0].title.plain,
        "Searched for TODO in src (interrupted)"
    );
}

#[test]
fn carries_presentation_badges_onto_command_and_exploration_titles() {
    let presentation = json!({
        "label": { "pending": "Running command", "completed": "Ran command" },
        "icon": { "glyph": "Terminal" },
        "badge": { "glyph": "SquareUnlock02", "label": "Outside of sandbox", "hint": "Outside of sandbox", "tone": "destructive" },
    });
    let badge = TitleDecoration::Badge {
        glyph: "SquareUnlock02".into(),
        label: "Outside of sandbox".into(),
        hint: "Outside of sandbox".into(),
        tone: PresentationBadgeTone::Destructive,
    };
    let plain_command = title(with(
        command_row(),
        json!({ "presentation": presentation.clone() }),
    ));
    assert_eq!(plain_command.decorations[0], badge);
    assert!(plain_command.plain.contains("(Outside of sandbox)"));
    let exploration = intent_titles(with(
        command_row(),
        json!({ "presentation": presentation.clone(), "activityIntents": [search_intent("TODO", Some("src")), list_intent(Some("test"))] }),
    ));
    assert_eq!(
        exploration[0].title.decorations,
        std::slice::from_ref(&badge)
    );
    assert!(exploration[1].title.decorations.is_empty());
    for (approval, status) in [
        (json!("waiting_for_approval"), "pending"),
        (json!("denied"), "interrupted"),
        (Value::Null, "pending"),
        (Value::Null, "error"),
        (Value::Null, "interrupted"),
        (Value::Null, "completed"),
    ] {
        let value = with(
            command_row(),
            json!({ "approvalStatus": approval, "status": status, "presentation": presentation.clone(), "activityIntents": [search_intent("TODO", Some("src"))] }),
        );
        let has_badge = |decorations: &[TitleDecoration]| {
            decorations
                .iter()
                .any(|decoration| matches!(decoration, TitleDecoration::Badge { .. }))
        };
        assert!(has_badge(&title(value.clone()).decorations), "{status}");
        assert!(
            has_badge(&intent_titles(value)[0].title.decorations),
            "{status}"
        );
    }
}

fn question_row(lifecycle: &str, questions: Vec<Value>, answers: Value) -> Value {
    with(
        base("question-1"),
        json!({
            "kind": "work",
            "workKind": "question",
            "status": "pending",
            "interactionId": "pi-1",
            "lifecycle": lifecycle,
            "questions": questions,
            "answers": answers,
            "statusReason": null,
        }),
    )
}

fn branch_question() -> Value {
    json!({
        "id": "branch",
        "prompt": "Which branch should I update?",
        "shortLabel": "Branch",
        "multiSelect": false,
        "allowFreeText": true,
        "options": [{ "value": "main", "label": "main" }, { "value": "release", "label": "release/1.0" }],
    })
}

fn tests_question() -> Value {
    json!({
        "id": "tests",
        "prompt": "Should I run the tests?",
        "shortLabel": "Tests",
        "multiSelect": false,
        "allowFreeText": true,
        "options": [{ "value": "yes", "label": "Yes" }, { "value": "no", "label": "No" }],
    })
}

#[test]
fn names_forms_while_waiting_and_once_submitted() {
    let form = |lifecycle: &str, title: Option<&str>| {
        let mut presentation = json!({
            "label": { "pending": "Waiting for Add secrets to .env", "completed": "Submitted Add secrets to .env" },
            "icon": { "glyph": "Toolbox" },
        });
        if let Some(title) = title {
            presentation["title"] = json!(title);
        }
        with(
            base("form-1"),
            json!({
                "kind": "work",
                "workKind": "form",
                "status": if lifecycle == "pending" { "pending" } else { "completed" },
                "interactionId": "pi-form",
                "pluginId": "secrets",
                "rendererId": "secret-request",
                "title": "Add secrets to .env",
                "lifecycle": lifecycle,
                "statusReason": null,
                "presentation": presentation,
                "payload": null,
            }),
        )
    };
    assert_eq!(
        title(form("pending", None)).plain,
        "Waiting for Add secrets to .env"
    );
    assert_eq!(
        title(form("submitted", None)).plain,
        "Submitted Add secrets to .env"
    );
    assert_eq!(
        title(form("submitted", Some("Added API_KEY to .env"))).plain,
        "Added API_KEY to .env"
    );
    let cancelled = title(form("cancelled", None));
    assert_eq!(
        cancelled.plain,
        "Submitted Add secrets to .env (interrupted)"
    );
    assert!(matches!(
        cancelled.decorations.as_slice(),
        [TitleDecoration::Status {
            status: StatusDecorationStatus::Interrupted,
            ..
        }]
    ));
}

#[test]
fn renders_question_rows() {
    assert_eq!(
        title(question_row(
            "pending",
            vec![branch_question()],
            Value::Null
        ))
        .plain,
        "Waiting for answer Which branch should I update?"
    );
    assert_eq!(
        title(question_row(
            "answered",
            vec![branch_question()],
            json!({ "branch": { "selected": ["release"], "freeText": "and tag it" } })
        ))
        .plain,
        "Answered Which branch should I update? — release/1.0, and tag it"
    );
    let pending = title(question_row(
        "pending",
        vec![branch_question(), tests_question()],
        Value::Null,
    ));
    assert_eq!(pending.plain, "Waiting for answers to 2 questions");
    assert_eq!(
        title(question_row(
            "answered",
            vec![branch_question(), tests_question()],
            json!({ "branch": { "selected": ["main"] }, "tests": { "selected": ["yes"] } })
        ))
        .plain,
        "Answered 2 questions"
    );
    let interrupted = title(question_row(
        "interrupted",
        vec![branch_question()],
        Value::Null,
    ));
    assert!(
        interrupted
            .plain
            .contains("Asked Which branch should I update?")
    );
    assert_eq!(
        interrupted.decorations,
        [status_decoration(
            StatusDecorationStatus::Interrupted,
            None,
            false
        )]
    );
}

#[test]
fn renders_workflow_and_background_task_titles() {
    let workflow = |task_type: &str, status: &str| {
        with(
            base("wf-1"),
            json!({
                "kind": "work",
                "workKind": "workflow",
                "status": status,
                "itemId": "item-1",
                "taskType": task_type,
                "workflowName": if task_type == "local_workflow" { json!("release-train") } else { Value::Null },
                "description": "Ship the release",
                "model": null,
                "taskStatus": "running",
                "workflow": {
                    "phases": [],
                    "agents": [
                        { "index": 1, "label": "a", "state": "done", "model": "m", "attempt": 1, "cached": false, "lastProgressAt": 1 },
                        { "index": 2, "label": "b", "state": "running", "model": "m", "attempt": 1, "cached": false, "lastProgressAt": 1 },
                    ],
                },
                "usage": null,
                "summary": null,
                "error": null,
                "completedAt": if status == "pending" { Value::Null } else { json!(4001) },
            }),
        )
    };
    assert_eq!(
        title(workflow("local_workflow", "pending")).plain,
        "Running workflow: release-train (1/2 agents)"
    );
    assert_eq!(
        title(workflow("local_bash", "completed")).plain,
        "Ran background command: Ship the release (4s)"
    );
    assert_eq!(
        title(workflow("local_agent", "error")).plain,
        "Background agent failed: Ship the release (4s)"
    );
}

#[test]
fn renders_remaining_work_kinds() {
    let plan = with(
        base("plan-1"),
        json!({
            "kind": "work",
            "workKind": "plan-steps",
            "status": "completed",
            "callId": "plan-call",
            "steps": [{ "step": "Write code", "status": "completed" }, { "step": "Test", "status": "pending" }],
            "explanation": null,
            "completedAt": 1,
        }),
    );
    assert_eq!(title(plan.clone()).plain, "Updated plan 1/2 steps done");
    let active_plan = with(
        plan,
        json!({ "status": "pending", "steps": [{ "step": "Run tests", "status": "active" }] }),
    );
    assert_eq!(title(active_plan).plain, "Updating plan Run tests");
    let image = with(
        base("image-1"),
        json!({
            "kind": "work",
            "workKind": "image-generation",
            "status": "error",
            "callId": "img",
            "prompt": "a cat",
            "path": null,
            "error": "boom",
            "transparentBackground": false,
            "completedAt": 9001,
        }),
    );
    assert_eq!(title(image).plain, "Image generation failed (error)");
    let extension = with(
        base("ext-1"),
        json!({
            "kind": "work",
            "workKind": "extension",
            "status": "completed",
            "callId": "ext",
            "extensionKind": "acme.deploy",
            "payload": {},
            "completedAt": 3001,
            "presentation": {
                "label": { "pending": "Deploying", "completed": "Deployed" },
                "icon": { "glyph": "acme/rocket" },
                "title": "staging",
                "badge": { "glyph": "Cloud", "label": "remote", "hint": "Runs remotely", "tone": "neutral" },
            },
        }),
    );
    let extension_title = title(extension);
    assert_eq!(extension_title.plain, "Deployed staging (remote) (3s)");
    assert!(!extension_title.segments[1].em);
    let search = with(
        base("search-1"),
        json!({
            "kind": "work",
            "workKind": "search",
            "status": "completed",
            "callId": "s",
            "mode": "path",
            "query": "*.rs",
            "path": "crates",
            "cmd": null,
            "completedAt": 1,
        }),
    );
    assert_eq!(title(search).plain, "Listed files in crates");
    let web_fetch_presented = with(
        web_fetch_row(),
        json!({ "presentation": { "label": { "pending": "Reading page", "completed": "Read page" }, "icon": { "glyph": "Globe" } } }),
    );
    assert_eq!(
        title(web_fetch_presented).plain,
        "Read page https://example.com/thread-view"
    );
    let tool_presented = with(
        tool_row(),
        json!({ "presentation": { "label": { "pending": "Looking up", "completed": "Looked up" }, "icon": { "glyph": "Search" } } }),
    );
    let tool_title = title(tool_presented);
    assert_eq!(tool_title.plain, "Looked up (2s)");
    assert!(tool_title.segments[0].truncate);
}

#[test]
fn titles_conversations_and_unknown_rows() {
    assert_eq!(title(assistant("a")).plain, "Assistant");
    assert_eq!(title(user("u", "hi")).plain, "User");
    assert_eq!(
        title(json!({ "kind": "future", "id": "x" })).plain,
        "Unsupported future row"
    );
    let unknown_work = view_work(with(
        base("w"),
        json!({ "kind": "work", "workKind": "hologram" }),
    ));
    assert!(
        matches!(&unknown_work, ViewRow::Work(work) if matches!(work.row, TimelineWorkRow::Unknown(_)))
    );
    assert_eq!(
        build_timeline_row_title(&unknown_work, DEFAULT).plain,
        "Unsupported work row (hologram)"
    );
}
