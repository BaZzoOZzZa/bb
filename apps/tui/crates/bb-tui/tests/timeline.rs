mod support;

use std::sync::Arc;

use bb_store::TurnDetailsKey;
use bb_tui::app::{Action, AppState, Effect, Focus, Mode, update};
use bb_tui::timeline::{ItemKind, Scroll};
use bb_wire::thread::ThreadDisplayStatus;
use bb_wire::timeline::{ActiveThinking, TimelineRow};
use crossterm::event::{KeyCode, KeyModifiers};
use serde_json::json;
use support::*;

const WIDTH: u16 = 100;
const HEIGHT: u16 = 28;

fn opened(rows: Vec<TimelineRow>, has_older: bool) -> AppState {
    let mut state = loaded_state(sample_sidebar());
    let snapshot = with_timeline(&state.snapshot, rows, has_older);
    update(&mut state, Action::Snapshot(snapshot));
    open_thread(&mut state);
    screen(&state);
    state
}

fn open_thread(state: &mut AppState) -> Vec<Effect> {
    let index = state
        .rows
        .iter()
        .position(|row| row.thread().is_some_and(|thread| thread.id == THREAD))
        .expect("thread row");
    state.selected = index;
    update(
        state,
        Action::Key(crossterm::event::KeyEvent::from(KeyCode::Enter)),
    )
}

fn screen(state: &AppState) -> Vec<String> {
    render_lines(state, WIDTH, HEIGHT)
}

fn main_text(state: &AppState) -> String {
    screen(state).join("\n")
}

fn select_row(state: &mut AppState, key: &str) {
    let timeline = state.timelines.get_mut(THREAD).expect("timeline");
    let suffix = format!("/{key}");
    let found = timeline
        .items
        .iter()
        .find(|item| item.key == key || item.key.ends_with(&suffix))
        .map(|item| item.key.clone())
        .unwrap_or_else(|| panic!("missing item {key}"));
    timeline.selected = Some(found);
}

fn expand_summaries(state: &mut AppState) {
    let keys: Vec<String> = state
        .timeline()
        .unwrap()
        .items
        .iter()
        .filter(|item| {
            matches!(&item.kind, ItemKind::Row(row) if matches!(*row.row, bb_view::ViewRow::Summary(_)) && !row.expanded)
        })
        .map(|item| item.key.clone())
        .collect();
    for key in keys {
        select_row(state, &key);
        press(state, KeyCode::Enter);
    }
}

fn opened_rich() -> AppState {
    let mut state = opened(rich_rows(), false);
    expand_summaries(&mut state);
    state
}

fn rich_rows() -> Vec<TimelineRow> {
    vec![
        user("u1", 1, "Please **fix** the parser and run the tests"),
        command(
            "c1",
            2,
            "cargo test -p parser",
            "running 3 tests\ntest ok\nall passed",
        ),
        row_from_json(json!({
            "kind": "work", "workKind": "file-change", "id": "f1", "threadId": THREAD,
            "turnId": "turn-0", "sourceSeqStart": 3, "sourceSeqEnd": 3,
            "startedAt": NOW_MS - 500_000, "createdAt": NOW_MS - 500_000,
            "status": "completed", "callId": "call-f1",
            "change": {
                "path": "src/parser.rs", "kind": "update", "movePath": null,
                "diff": "@@ -10,3 +10,3 @@ fn parse()\n let a = 1;\n-let b = 2;\n+let b = 3;\n let c = 4;\n",
                "diffStats": {"added": 1, "removed": 1}
            },
            "stdout": null, "stderr": null, "approvalStatus": null
        })),
        row_from_json(json!({
            "kind": "system", "systemKind": "operation", "operationKind": "reasoning",
            "id": "r1", "threadId": THREAD, "turnId": "turn-0",
            "sourceSeqStart": 4, "sourceSeqEnd": 4,
            "startedAt": NOW_MS - 400_000, "createdAt": NOW_MS - 400_000,
            "title": "Reasoning", "detail": "Consider the **edge** cases first.",
            "status": "completed", "reasoningId": "rs-1", "completedAt": NOW_MS - 397_000
        })),
        assistant(
            "a1",
            5,
            "Fixed it. See [the docs](https://example.com/parser).\n\n```rust\nfn parse() -> u8 { 3 }\n```\n\n| Case | Result |\n|------|--------|\n| empty | ok |\n| nested | ok |",
        ),
    ]
}

#[test]
fn renders_conversation_markdown_and_work_rows() {
    let state = opened(rich_rows(), false);
    let text = main_text(&state);
    assert!(text.contains("▸ ≡ Ran 1 command, edited 1 file"), "{text}");
    let mut state = state;
    expand_summaries(&mut state);
    let text = main_text(&state);
    assert!(
        text.contains("❯ Please fix the parser and run the tests"),
        "{text}"
    );
    assert!(text.contains("$ Ran cargo test -p parser"), "{text}");
    assert!(text.contains("✎ Edited parser.rs +1 -1"), "{text}");
    assert!(text.contains("✻ Thought (3s)"), "{text}");
    assert!(
        text.contains("● Fixed it. See the docs (https://example.com/parser)."),
        "{text}"
    );
    assert!(text.contains("▏ fn parse() -> u8 { 3 }"), "{text}");
    assert!(text.contains("│ empty  │ ok     │"), "{text}");
    assert!(!text.contains("Unsupported"), "{text}");
    let timeline = state.timeline().expect("timeline");
    assert_eq!(timeline.scroll, Scroll::Bottom);
}

#[test]
fn expanding_rows_shows_output_and_diff() {
    let mut state = opened_rich();
    assert_eq!(state.focus, Focus::Main);
    select_row(&mut state, "c1");
    press(&mut state, KeyCode::Enter);
    let text = main_text(&state);
    assert!(text.contains("▾ $ Ran cargo test -p parser"), "{text}");
    assert!(text.contains("  $ cargo test -p parser"), "{text}");
    assert!(text.contains("  all passed"), "{text}");

    select_row(&mut state, "f1");
    press(&mut state, KeyCode::Char('l'));
    let text = main_text(&state);
    assert!(text.contains("@@ -10,3 +10,3 @@ fn parse()"), "{text}");
    assert!(text.contains("11 -let b = 2;"), "{text}");
    assert!(text.contains("11 +let b = 3;"), "{text}");

    press(&mut state, KeyCode::Char('h'));
    let text = main_text(&state);
    assert!(!text.contains("+let b = 3;"), "{text}");
    assert!(text.contains("▸ ✎ Edited parser.rs"), "{text}");
}

#[test]
fn diff_lines_are_colored() {
    let mut state = opened_rich();
    select_row(&mut state, "f1");
    press(&mut state, KeyCode::Enter);
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(WIDTH, HEIGHT)).unwrap();
    terminal
        .draw(|frame| bb_tui::ui::render(&state, frame))
        .unwrap();
    let lines = buffer_lines(&terminal);
    let y = lines
        .iter()
        .position(|line| line.contains("+let b = 3;"))
        .expect("added line");
    let x = lines[y].find("+let").unwrap();
    let column = lines[y][..x].chars().count() as u16;
    let cell = terminal.backend().buffer()[(column, y as u16)].clone();
    assert_eq!(cell.fg, ratatui::style::Color::Green);
}

#[test]
fn turn_rows_load_children_lazily() {
    let rows = vec![
        user("u1", 100, "Start"),
        turn("t1", "turn-1", 101, 140),
        assistant("a1", 141, "All done"),
    ];
    let mut state = opened(rows, false);
    assert!(
        main_text(&state).contains("▸ ◦ Worked for"),
        "{}",
        main_text(&state)
    );
    select_row(&mut state, "t1");
    let effects = press(&mut state, KeyCode::Enter);
    let key = TurnDetailsKey {
        thread_id: THREAD.into(),
        turn_id: "turn-1".into(),
        source_seq_start: 101,
        source_seq_end: 140,
        item_id: None,
    };
    assert_eq!(effects, vec![Effect::LoadTurnDetails(key.clone())]);
    assert!(main_text(&state).contains("Loading…"));
    assert!(
        press(&mut state, KeyCode::Char('k')).is_empty(),
        "turn details are requested once"
    );
    let snapshot = with_turn_details(
        &state.snapshot,
        key,
        vec![
            command("t1-c1", 102, "ls -la", "a\nb"),
            command("t1-c2", 103, "make build", "ok"),
            assistant("t1-a", 104, "Intermediate note"),
        ],
    );
    update(&mut state, Action::Snapshot(snapshot));
    let text = main_text(&state);
    assert!(!text.contains("Loading…"), "{text}");
    assert!(text.contains("▸ ≡ Ran 2 commands"), "{text}");
    assert!(text.contains("● Intermediate note"), "{text}");
}

#[test]
fn deferred_content_loads_on_expand() {
    let deferred = row_from_json(json!({
        "kind": "work", "workKind": "command", "id": "c9", "threadId": THREAD,
        "turnId": "turn-9", "sourceSeqStart": 9, "sourceSeqEnd": 9,
        "startedAt": NOW_MS - 100_000, "createdAt": NOW_MS - 100_000,
        "status": "completed", "callId": "call-9", "command": "cat big.log",
        "cwd": null, "source": null, "output": "", "exitCode": 0,
        "completedAt": NOW_MS - 99_000, "approvalStatus": null,
        "activityIntents": [], "contentDeferred": true
    }));
    let mut state = opened(vec![user("u1", 1, "go"), deferred.clone()], false);
    select_row(&mut state, "c9");
    let effects = press(&mut state, KeyCode::Enter);
    assert!(
        matches!(&effects[..], [Effect::LoadDeferred { thread_id, row }] if thread_id == THREAD && row.id() == Some("c9")),
        "{effects:?}"
    );
    assert!(main_text(&state).contains("Loading…"));
    let mut resolved = deferred;
    if let TimelineRow::Work(bb_wire::timeline::TimelineWorkRow::Command(row)) = &mut resolved {
        row.output = "line one\nline two".into();
        row.content_deferred = false;
    }
    let key = TurnDetailsKey::for_deferred_row(THREAD, &{
        let TimelineRow::Work(work) = &resolved else {
            unreachable!()
        };
        let mut row = work.clone();
        if let bb_wire::timeline::TimelineWorkRow::Command(command) = &mut row {
            command.content_deferred = true;
        }
        TimelineRow::Work(row)
    })
    .expect("key");
    let snapshot = with_turn_details(&state.snapshot, key, vec![resolved]);
    update(&mut state, Action::Snapshot(snapshot));
    let text = main_text(&state);
    assert!(text.contains("line two"), "{text}");
    assert!(!text.contains("Loading…"), "{text}");
}

fn long_rows(count: i64) -> Vec<TimelineRow> {
    (0..count)
        .map(|index| {
            if index % 2 == 0 {
                user(
                    &format!("u{index}"),
                    index + 10,
                    &format!("Question number {index}"),
                )
            } else {
                assistant(
                    &format!("a{index}"),
                    index + 10,
                    &format!("Answer number {index}"),
                )
            }
        })
        .collect()
}

#[test]
fn sticky_bottom_follows_until_user_scrolls_up() {
    let mut rows = long_rows(30);
    let mut state = opened(rows.clone(), false);
    assert!(main_text(&state).contains("Answer number 29"));

    rows.push(user("u-new", 100, "Fresh question"));
    let snapshot = with_timeline(&state.snapshot, rows.clone(), false);
    update(&mut state, Action::Snapshot(snapshot));
    let text = main_text(&state);
    assert!(text.contains("Fresh question"), "{text}");

    press_mod(&mut state, KeyCode::Char('u'), KeyModifiers::CONTROL);
    let timeline = state.timeline().unwrap();
    assert!(matches!(timeline.scroll, Scroll::Anchor { .. }));
    let before = screen(&state);
    assert!(!before.join("\n").contains("Fresh question"));

    rows.push(assistant("a-new", 101, "Fresh answer"));
    let snapshot = with_timeline(&state.snapshot, rows.clone(), false);
    update(&mut state, Action::Snapshot(snapshot));
    let after = screen(&state);
    assert_eq!(before[3..10], after[3..10], "view must not move");
    assert!(after.join("\n").contains("new activity"));

    press(&mut state, KeyCode::Char('G'));
    let text = main_text(&state);
    assert!(text.contains("Fresh answer"), "{text}");
    assert!(!text.contains("new activity"), "{text}");
    assert_eq!(state.timeline().unwrap().scroll, Scroll::Bottom);
}

#[test]
fn scrolling_down_to_the_end_re_engages_follow() {
    let mut state = opened(long_rows(30), false);
    press_mod(&mut state, KeyCode::Char('u'), KeyModifiers::CONTROL);
    assert!(matches!(
        state.timeline().unwrap().scroll,
        Scroll::Anchor { .. }
    ));
    press_mod(&mut state, KeyCode::Char('d'), KeyModifiers::CONTROL);
    press_mod(&mut state, KeyCode::Char('d'), KeyModifiers::CONTROL);
    assert_eq!(state.timeline().unwrap().scroll, Scroll::Bottom);
}

#[test]
fn reaching_the_top_loads_older_rows_and_keeps_position() {
    let rows = long_rows(30);
    let mut state = opened(rows.clone(), true);
    screen(&state);
    let mut effects = Vec::new();
    for _ in 0..10 {
        effects.extend(press(&mut state, KeyCode::PageUp));
    }
    assert!(
        effects
            .iter()
            .any(|effect| *effect == Effect::LoadOlder(THREAD.into())),
        "{effects:?}"
    );
    let text = main_text(&state);
    assert!(
        text.contains("Older messages") || text.contains("Loading older"),
        "{text}"
    );
    assert!(text.contains("Question number 0"), "{text}");

    let mut older: Vec<TimelineRow> = (0..10)
        .map(|index| {
            user(
                &format!("old{index}"),
                index,
                &format!("Old message {index}"),
            )
        })
        .collect();
    older.extend(rows);
    let snapshot = with_timeline(&state.snapshot, older, false);
    update(&mut state, Action::Snapshot(snapshot));
    let text = main_text(&state);
    assert!(text.contains("Question number 0"), "{text}");
    assert!(!text.contains("Old message 0"), "{text}");
    assert!(!state.timeline().unwrap().unseen);
    press_mod(&mut state, KeyCode::Char('u'), KeyModifiers::CONTROL);
    assert!(main_text(&state).contains("Old message 9"));
}

#[test]
fn growing_stream_keeps_settled_lines_stable() {
    let mut state = loaded_state(sample_sidebar());
    let mut sidebar = sample_sidebar();
    for project in &mut sidebar.projects {
        for entry in &mut project.threads {
            if entry.thread.id == THREAD {
                entry.runtime.display_status = ThreadDisplayStatus::Active;
            }
        }
    }
    update(&mut state, Action::Snapshot(snapshot_with(sidebar)));
    let mut text = String::from("First paragraph is settled.\n\nSecond paragraph is still str");
    let rows = |text: &str| vec![user("u1", 1, "Write"), assistant("a1", 2, text)];
    let snapshot = with_timeline(&state.snapshot, rows(&text), false);
    update(&mut state, Action::Snapshot(snapshot));
    open_thread(&mut state);
    let first = screen(&state);
    let joined = first.join("\n");
    assert!(joined.contains("● First paragraph is settled."), "{joined}");
    assert!(joined.contains("Working…"), "{joined}");
    let anchor = first
        .iter()
        .position(|line| line.contains("First paragraph"))
        .unwrap();

    text.push_str("eaming with **bold");
    let snapshot = with_timeline(&state.snapshot, rows(&text), false);
    update(&mut state, Action::Snapshot(snapshot));
    let second = screen(&state);
    let joined = second.join("\n");
    assert!(
        joined.contains("Second paragraph is still streaming with bold"),
        "{joined}"
    );
    assert!(!joined.contains("**"), "{joined}");
    let moved = second
        .iter()
        .position(|line| line.contains("First paragraph"))
        .unwrap();
    assert_eq!(moved, anchor, "settled text stays put while the tail grows");
}

#[test]
fn active_thinking_shows_spinner_and_text() {
    let mut state = loaded_state(sample_sidebar());
    let mut response = timeline_response(vec![user("u1", 1, "Think hard")], false);
    response.active_thinking = Some(ActiveThinking {
        id: "th".into(),
        text: "Considering options\nWeighing the parser rewrite".into(),
        started_at: NOW_MS - 12_000,
        updated_at: NOW_MS,
    });
    let mut snapshot = (*state.snapshot).clone();
    snapshot
        .timelines
        .insert(THREAD.into(), timeline_entry(response));
    update(&mut state, Action::Snapshot(Arc::new(snapshot)));
    open_thread(&mut state);
    let text = main_text(&state);
    assert!(text.contains("Thinking… (12s)"), "{text}");
    assert!(text.contains("Weighing the parser rewrite"), "{text}");
}

#[test]
fn copy_and_open_selected_row() {
    let mut state = opened_rich();
    state.timelines.get_mut(THREAD).unwrap().selected = None;
    assert!(press(&mut state, KeyCode::Char('y')).is_empty());
    select_row(&mut state, "a1");
    let effects = press(&mut state, KeyCode::Char('y'));
    assert!(
        matches!(&effects[..], [Effect::Copy(text)] if text.starts_with("Fixed it. See [the docs]")),
        "{effects:?}"
    );
    let effects = press(&mut state, KeyCode::Char('o'));
    assert_eq!(
        effects,
        vec![Effect::OpenExternal("https://example.com/parser".into())]
    );

    select_row(&mut state, "c1");
    let effects = press(&mut state, KeyCode::Char('y'));
    assert!(
        matches!(&effects[..], [Effect::Copy(text)] if text.contains("$ cargo test -p parser") && text.contains("all passed")),
        "{effects:?}"
    );
}

#[test]
fn open_picker_lists_multiple_targets() {
    let mut state = opened(
        vec![assistant(
            "a1",
            1,
            "Links: https://one.dev and [two](https://two.dev/page)",
        )],
        false,
    );
    select_row(&mut state, "a1");
    assert!(press(&mut state, KeyCode::Char('o')).is_empty());
    assert!(matches!(&state.mode, Mode::OpenPicker { targets, .. } if targets.len() == 2));
    let text = main_text(&state);
    assert!(text.contains("1 https://two.dev/page"), "{text}");
    assert!(text.contains("2 https://one.dev"), "{text}");
    let effects = press(&mut state, KeyCode::Char('2'));
    assert_eq!(
        effects,
        vec![Effect::OpenExternal("https://one.dev".into())]
    );
    assert_eq!(state.mode, Mode::Normal);
}

#[test]
fn message_jumps_and_selection_scrolls_into_view() {
    let mut state = opened(long_rows(30), false);
    screen(&state);
    press(&mut state, KeyCode::Char('g'));
    let text = main_text(&state);
    assert!(text.contains("Question number 0"), "{text}");
    press(&mut state, KeyCode::Char(']'));
    let selected = state.timeline().unwrap().selected.clone();
    assert_eq!(selected.as_deref(), Some("u2"));
    for _ in 0..12 {
        press(&mut state, KeyCode::Char('j'));
    }
    let timeline = state.timeline().unwrap();
    let selected = timeline.selected_index().unwrap();
    let (width, height) = timeline.viewport_size();
    assert!(
        timeline
            .layout(width, height)
            .iter()
            .any(|placement| placement.index == selected)
    );
}

#[test]
fn renders_every_row_kind_from_the_shared_fixture() {
    let rows: Vec<TimelineRow> = serde_json::from_str(include_str!(
        "../../bb-view/tests/fixtures/rich_timeline.json"
    ))
    .expect("fixture");
    let rows: Vec<TimelineRow> = rows
        .into_iter()
        .map(|row| {
            let mut value = serde_json::to_value(row).unwrap();
            value["threadId"] = json!(THREAD);
            serde_json::from_value(value).unwrap()
        })
        .collect();
    let mut state = opened(rows, false);
    let keys: Vec<String> = state
        .timeline()
        .unwrap()
        .items
        .iter()
        .filter(|item| matches!(item.kind, ItemKind::Row(_)))
        .map(|item| item.key.clone())
        .collect();
    for key in keys {
        select_row(&mut state, &key);
        press(&mut state, KeyCode::Char('l'));
    }
    for width in [40u16, 80, 140] {
        for line in render_lines(&state, width, 200) {
            assert!(unicode_width::UnicodeWidthStr::width(line.as_str()) <= usize::from(width));
        }
    }
    let timeline = state.timeline().unwrap();
    assert!(timeline.items.len() > 30, "{}", timeline.items.len());
}

#[test]
fn virtualized_layout_only_renders_visible_rows() {
    let state = opened(long_rows(3_000), false);
    screen(&state);
    let timeline = state.timeline().unwrap();
    assert_eq!(timeline.items.len(), 3_000);
    assert!(timeline.cached_rows() < 40, "{}", timeline.cached_rows());
}

#[test]
fn background_workflows_show_activity_while_idle() {
    let mut state = loaded_state(sample_sidebar());
    let mut response = timeline_response(vec![user("u1", 1, "Run the workflow")], false);
    response.active_workflows = vec![bb_wire::timeline::WorkflowWorkRow {
        base: base("wf", 2),
        workflow_name: Some("nightly".into()),
        description: "Nightly checks".into(),
        ..bb_wire::timeline::WorkflowWorkRow::default()
    }];
    let mut snapshot = (*state.snapshot).clone();
    snapshot
        .timelines
        .insert(THREAD.into(), timeline_entry(response));
    update(&mut state, Action::Snapshot(Arc::new(snapshot)));
    open_thread(&mut state);
    let text = main_text(&state);
    assert!(text.contains("Running nightly… (9m 58s)"), "{text}");
}
