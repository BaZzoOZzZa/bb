mod support;

use std::sync::Arc;
use std::time::Duration;

use bb_store::{ConnectionState, StoreSnapshot};
use bb_tui::api::Mutation;
use bb_tui::app::{Action, AppState, Effect, Focus, Mode, ToastLevel, update};
use bb_tui::sidebar::{RowKey, SidebarRow};
use bb_wire::system::{AppKeybindingOverride, AppShortcut};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use support::*;

fn press(state: &mut AppState, code: KeyCode) -> Vec<Effect> {
    update(state, Action::Key(KeyEvent::from(code)))
}

fn press_mod(state: &mut AppState, code: KeyCode, modifiers: KeyModifiers) -> Vec<Effect> {
    update(state, Action::Key(KeyEvent::new(code, modifiers)))
}

fn type_text(state: &mut AppState, text: &str) {
    type_chars(state, text);
}

fn selected_key(state: &AppState) -> Option<RowKey> {
    state.selected_row().map(SidebarRow::key)
}

fn thread_key(id: &str) -> Option<RowKey> {
    Some(RowKey::Thread(id.into()))
}

fn screen(state: &AppState) -> String {
    render_lines(state, 100, 16).join("\n")
}

#[test]
fn sidebar_renders_tree_glyphs_times_and_status_bar() {
    let state = loaded_state(sample_sidebar());
    let lines = render_lines(&state, 100, 12);
    let text = lines.join("\n");
    assert!(lines[1].contains("▾ alpha"), "{text}");
    assert!(lines[1].contains("3┃"), "{text}");
    assert!(lines[2].contains(" ? Needs approval"), "{text}");
    assert!(lines[2].contains("3m"), "{text}");
    assert!(lines[3].contains(" ● Fix flaky test"), "{text}");
    assert!(lines[3].contains("30m"), "{text}");
    assert!(lines[4].contains("Write docs"), "{text}");
    assert!(lines[4].contains("1d"), "{text}");
    assert!(lines[5].contains("▾ beta"), "{text}");
    assert!(lines[6].contains("Refactor parser"), "{text}");
    assert!(!text.contains("Personal"), "{text}");
    let status = &lines[lines.len() - 2];
    assert!(status.contains("http://127.0.0.1:4000/"), "{status}");
    assert!(status.contains("● connected"), "{status}");
    assert!(status.contains("⚑ 2"), "{status}");
    assert!(status.contains("*threads*"), "{status}");
    assert!(status.trim_end().ends_with("<N>"), "{status}");
    let echo = lines.last().unwrap();
    assert!(echo.contains("SPC leader"), "{echo}");
    assert!(text.contains("Select a thread and press RET"), "{text}");
}

#[test]
fn first_load_selects_first_thread_and_vim_keys_move() {
    let mut state = loaded_state(sample_sidebar());
    assert_eq!(selected_key(&state), thread_key("thr_wait"));
    press(&mut state, KeyCode::Char('j'));
    assert_eq!(selected_key(&state), thread_key("thr_unread"));
    press(&mut state, KeyCode::Char('k'));
    press(&mut state, KeyCode::Char('k'));
    assert_eq!(selected_key(&state), Some(RowKey::Group("proj_a".into())));
    press(&mut state, KeyCode::Char('k'));
    assert_eq!(selected_key(&state), Some(RowKey::Group("proj_a".into())));
    press_mod(&mut state, KeyCode::Char('G'), KeyModifiers::SHIFT);
    assert_eq!(selected_key(&state), thread_key("thr_other"));
    keys(&mut state, "g g");
    assert_eq!(state.selected, 0);
    keys(&mut state, "3 j");
    assert_eq!(selected_key(&state), thread_key("thr_old"));
    keys(&mut state, "2 G");
    assert_eq!(selected_key(&state), thread_key("thr_wait"));
    press(&mut state, KeyCode::Char('g'));
    assert_eq!(state.keys.showcmd(), "g");
    press(&mut state, KeyCode::Esc);
    assert!(!state.keys.is_pending());
    assert_eq!(selected_key(&state), thread_key("thr_wait"));
}

#[test]
fn enter_opens_thread_marks_unread_read_and_focuses_main() {
    let mut state = loaded_state(sample_sidebar());
    press(&mut state, KeyCode::Char('j'));
    let effects = without_composer_loads(press(&mut state, KeyCode::Enter));
    assert_eq!(
        effects,
        vec![
            Effect::WatchThread(Some("thr_unread".into())),
            Effect::Mutate(Mutation::MarkRead {
                thread_id: "thr_unread".into()
            }),
        ]
    );
    assert_eq!(state.focus, Focus::Sidebar);
    keys(&mut state, "g o");
    assert_eq!(state.focus, Focus::Main);
    assert_eq!(state.open_thread.as_deref(), Some("thr_unread"));
    let text = screen(&state);
    assert!(text.contains("┏ Fix flaky test ━"), "{text}");
    assert!(
        text.contains("● Unread thread succeeded · alpha · codex · unread"),
        "{text}"
    );

    press(&mut state, KeyCode::Char('j'));
    assert_eq!(selected_key(&state), thread_key("thr_unread"));
    press(&mut state, KeyCode::Esc);
    assert_eq!(state.focus, Focus::Main);
    press_mod(&mut state, KeyCode::Char('h'), KeyModifiers::CONTROL);
    assert_eq!(state.focus, Focus::Sidebar);
    press(&mut state, KeyCode::Char('k'));
    let effects = without_composer_loads(press(&mut state, KeyCode::Enter));
    assert_eq!(effects, vec![Effect::WatchThread(Some("thr_wait".into()))]);
}

#[test]
fn enter_on_group_and_h_l_collapse_and_expand() {
    let mut state = loaded_state(sample_sidebar());
    press(&mut state, KeyCode::Char('h'));
    assert_eq!(selected_key(&state), Some(RowKey::Group("proj_a".into())));
    press(&mut state, KeyCode::Char('h'));
    assert!(state.collapsed.contains("proj_a"));
    let lines = render_lines(&state, 100, 12);
    assert!(lines[1].contains("▸ alpha"), "{lines:?}");
    assert!(lines[1].contains("●2 3"), "{lines:?}");
    assert!(lines[2].contains("▾ beta"), "{lines:?}");
    press(&mut state, KeyCode::Enter);
    assert!(!state.collapsed.contains("proj_a"));
    press(&mut state, KeyCode::Tab);
    assert!(state.collapsed.contains("proj_a"));
    press(&mut state, KeyCode::Char('l'));
    assert!(!state.collapsed.contains("proj_a"));
    keys(&mut state, "z a");
    assert!(state.collapsed.contains("proj_a"));
    keys(&mut state, "z R");
    assert!(state.collapsed.is_empty());
    keys(&mut state, "z M");
    assert!(state.collapsed.contains("proj_a") && state.collapsed.contains("proj_b"));
}

#[test]
fn child_threads_expand_under_parent() {
    let mut sidebar = sample_sidebar();
    let mut child = thread("thr_child", "proj_a", "Subagent", 1);
    child.thread.parent_thread_id = Some("thr_wait".into());
    sidebar.projects[0].threads.push(child);
    let mut state = loaded_state(sidebar);
    let lines = render_lines(&state, 100, 12);
    assert!(lines[2].contains("▸? Needs approval"), "{lines:?}");
    assert!(lines[2].contains("+1 3m"), "{lines:?}");
    press(&mut state, KeyCode::Char('l'));
    let lines = render_lines(&state, 100, 12);
    assert!(lines[2].contains("▾? Needs approval"), "{lines:?}");
    assert!(lines[3].contains("     Subagent"), "{lines:?}");
    press(&mut state, KeyCode::Char('j'));
    assert_eq!(selected_key(&state), thread_key("thr_child"));
    press(&mut state, KeyCode::Char('h'));
    assert_eq!(selected_key(&state), thread_key("thr_wait"));
    press(&mut state, KeyCode::Char('h'));
    assert!(!state.expanded_threads.contains("thr_wait"));
}

#[test]
fn slash_filters_threads_and_esc_clears() {
    let mut state = loaded_state(sample_sidebar());
    keys(&mut state, "g /");
    assert!(matches!(state.mode, Mode::Filter(_)));
    type_text(&mut state, "parser");
    assert_eq!(state.filter, "parser");
    assert_eq!(selected_key(&state), thread_key("thr_other"));
    let lines = render_lines(&state, 100, 12);
    assert!(lines[1].contains("/parser"), "{lines:?}");
    assert!(lines[2].contains("▾ beta"), "{lines:?}");
    assert!(lines[3].contains("Refactor parser"), "{lines:?}");
    assert!(!lines.join("\n").contains("alpha"), "{lines:?}");
    press(&mut state, KeyCode::Enter);
    assert_eq!(state.mode, Mode::Normal);
    assert_eq!(state.filter, "parser");
    press(&mut state, KeyCode::Char('q'));
    assert!(!state.quit);
    assert_eq!(
        state.keys.echo.as_ref().map(|echo| echo.text.as_str()),
        Some("Quit with SPC q q or :q")
    );
    keys(&mut state, "SPC q q");
    assert!(state.quit);

    let mut state = loaded_state(sample_sidebar());
    keys(&mut state, "g /");
    type_text(&mut state, "zzzz");
    assert!(screen(&state).contains("No matching threads"));
    press(&mut state, KeyCode::Esc);
    assert_eq!(state.filter, "");
    assert_eq!(state.rows.len(), 6);
    keys(&mut state, ": f i l t e r SPC d o c s RET");
    assert_eq!(state.filter, "docs");
    press(&mut state, KeyCode::Esc);
    assert_eq!(state.filter, "");
    keys(&mut state, "g /");
    type_text(&mut state, "docs");
    press(&mut state, KeyCode::Enter);
    press(&mut state, KeyCode::Esc);
    assert_eq!(state.filter, "");
    assert_eq!(selected_key(&state), thread_key("thr_old"));
}

#[test]
fn rename_inline_submits_mutation_and_cancel_does_not() {
    let mut state = loaded_state(sample_sidebar());
    press(&mut state, KeyCode::Char('R'));
    let Mode::Rename { thread_id, input } = &state.mode else {
        panic!("rename mode");
    };
    assert_eq!(thread_id, "thr_wait");
    assert_eq!(input.text(), "Needs approval");
    press_mod(&mut state, KeyCode::Char('u'), KeyModifiers::CONTROL);
    type_text(&mut state, "Ship it q");
    let lines = render_lines(&state, 100, 12);
    assert!(lines[2].contains("? Ship it q"), "{lines:?}");
    assert!(!state.quit);
    let effects = press(&mut state, KeyCode::Enter);
    assert_eq!(
        effects,
        vec![Effect::Mutate(Mutation::Rename {
            thread_id: "thr_wait".into(),
            title: "Ship it q".into()
        })]
    );
    assert_eq!(state.mode, Mode::Normal);

    keys(&mut state, "SPC b R");
    type_text(&mut state, "!!");
    assert!(press(&mut state, KeyCode::Esc).is_empty());
    press(&mut state, KeyCode::Char('R'));
    assert!(press(&mut state, KeyCode::Enter).is_empty());
    assert_eq!(
        keys(&mut state, ": r e n SPC N e w SPC n a m e RET"),
        vec![Effect::Mutate(Mutation::Rename {
            thread_id: "thr_wait".into(),
            title: "New name".into()
        })]
    );
}

#[test]
fn archive_undo_pin_and_read_toggles() {
    let mut state = loaded_state(sample_sidebar());
    assert!(press(&mut state, KeyCode::Char('d')).is_empty());
    let archive = press(&mut state, KeyCode::Char('d'));
    assert_eq!(
        archive,
        vec![Effect::Mutate(Mutation::Archive {
            thread_id: "thr_wait".into()
        })]
    );
    let undo = press(&mut state, KeyCode::Char('u'));
    assert_eq!(
        undo,
        vec![Effect::Mutate(Mutation::Unarchive {
            thread_id: "thr_wait".into()
        })]
    );
    assert!(press(&mut state, KeyCode::Char('u')).is_empty());
    assert!(state.toasts.iter().any(|t| t.message == "Nothing to undo"));
    assert_eq!(
        press(&mut state, KeyCode::Char('x')),
        vec![Effect::Mutate(Mutation::Archive {
            thread_id: "thr_wait".into()
        })]
    );
    assert_eq!(
        keys(&mut state, "SPC b u"),
        vec![Effect::Mutate(Mutation::Unarchive {
            thread_id: "thr_wait".into()
        })]
    );

    assert_eq!(
        press(&mut state, KeyCode::Char('+')),
        vec![Effect::Mutate(Mutation::Pin {
            thread_id: "thr_wait".into()
        })]
    );
    assert_eq!(
        press(&mut state, KeyCode::Char('!')),
        vec![Effect::Mutate(Mutation::MarkUnread {
            thread_id: "thr_wait".into()
        })]
    );
    press(&mut state, KeyCode::Char('j'));
    assert_eq!(
        keys(&mut state, "SPC b !"),
        vec![Effect::Mutate(Mutation::MarkRead {
            thread_id: "thr_unread".into()
        })]
    );

    let mut sidebar = sample_sidebar();
    sidebar.projects[1].threads[0].thread.pinned_at = Some(1);
    let mut state = loaded_state(sidebar);
    assert_eq!(selected_key(&state), thread_key("thr_other"));
    assert!(screen(&state).contains("▾ Pinned"));
    assert_eq!(
        keys(&mut state, "SPC b m"),
        vec![Effect::Mutate(Mutation::Unpin {
            thread_id: "thr_other".into()
        })]
    );
}

#[test]
fn mutation_results_show_toasts_that_expire() {
    let mut state = loaded_state(sample_sidebar());
    let start = state.clock;
    let effects = update(
        &mut state,
        Action::MutationFinished {
            mutation: Mutation::Pin {
                thread_id: "thr_old".into(),
            },
            result: Ok(bb_tui::api::MutationReply::Done),
        },
    );
    assert_eq!(effects, vec![Effect::RefreshSidebar]);
    press(&mut state, KeyCode::Char('x'));
    update(
        &mut state,
        Action::MutationFinished {
            mutation: Mutation::Archive {
                thread_id: "thr_wait".into(),
            },
            result: Err("boom".into()),
        },
    );
    assert_eq!(state.last_archived, None);
    let text = screen(&state);
    assert!(text.contains("Pinned: Write docs"), "{text}");
    assert!(text.contains("Failed to archive: boom"), "{text}");

    update(
        &mut state,
        Action::Tick {
            now: start + Duration::from_secs(5),
            now_ms: NOW_MS,
        },
    );
    assert_eq!(state.toasts.len(), 1);
    assert_eq!(state.toasts[0].level, ToastLevel::Error);
    update(
        &mut state,
        Action::Tick {
            now: start + Duration::from_secs(9),
            now_ms: NOW_MS,
        },
    );
    assert!(state.toasts.is_empty());

    let silent = update(
        &mut state,
        Action::MutationFinished {
            mutation: Mutation::MarkRead {
                thread_id: "thr_old".into(),
            },
            result: Ok(bb_tui::api::MutationReply::Done),
        },
    );
    assert_eq!(silent, vec![Effect::RefreshSidebar]);
    assert!(state.toasts.is_empty());
}

#[test]
fn disconnect_shows_banner_and_status() {
    let mut state = loaded_state(sample_sidebar());
    let mut snapshot = StoreSnapshot::clone(&state.snapshot);
    snapshot.connection = ConnectionState::Disconnected {
        retry_in: Some(Duration::from_secs(4)),
        error: Some("connection refused".into()),
    };
    update(&mut state, Action::Snapshot(Arc::new(snapshot)));
    let lines = render_lines(&state, 100, 12);
    assert_eq!(
        lines[0],
        " Disconnected from server: connection refused · retrying in 4s"
    );
    assert!(lines[lines.len() - 2].contains("✗ disconnected"));

    let mut snapshot = StoreSnapshot::clone(&state.snapshot);
    snapshot.connection = ConnectionState::Connecting { reconnect: true };
    update(&mut state, Action::Snapshot(Arc::new(snapshot)));
    let lines = render_lines(&state, 100, 12);
    assert!(lines[0].starts_with("┏"), "{lines:?}");
    assert!(lines[lines.len() - 2].contains("◌ reconnecting"));
}

#[test]
fn sidebar_load_error_and_loading_states() {
    let mut state = loaded_state(sample_sidebar());
    let mut snapshot = StoreSnapshot::default();
    snapshot.sidebar.error = Some(bb_store::FetchError::new("HTTP 500"));
    update(&mut state, Action::Snapshot(Arc::new(snapshot)));
    let lines = render_lines(&state, 80, 8);
    assert_eq!(lines[0], " Failed to load threads: HTTP 500");
    assert!(lines[2].contains("No threads"), "{lines:?}");

    update(
        &mut state,
        Action::Snapshot(Arc::new(StoreSnapshot::default())),
    );
    assert!(screen(&state).contains("Loading…"));
}

#[test]
fn focus_cycles_through_visible_panes() {
    let mut state = loaded_state(sample_sidebar());
    keys(&mut state, "C-w w");
    assert_eq!(state.focus, Focus::Main);
    keys(&mut state, "C-w C-w");
    assert_eq!(state.focus, Focus::Sidebar);
    keys(&mut state, "SPC o i");
    keys(&mut state, "C-w W");
    assert_eq!(state.focus, Focus::Panel);
    let text = screen(&state);
    assert!(text.contains("┏ 1 Info │ 2 Diff │ 3 File │"), "{text}");
    keys(&mut state, "C-w p");
    assert_eq!(state.focus, Focus::Sidebar);
    keys(&mut state, "SPC o i");
    assert!(!state.panel_visible);
    keys(&mut state, "C-l");
    assert_eq!(state.focus, Focus::Main);
    keys(&mut state, "SPC o p");
    assert!(!state.sidebar_visible);
    keys(&mut state, "C-w w");
    assert_eq!(state.focus, Focus::Main);
    assert!(!screen(&state).contains("alpha"));
    keys(&mut state, "SPC s f");
    assert!(state.sidebar_visible);
    assert_eq!(state.focus, Focus::Sidebar);
}

#[test]
fn thread_next_previous_and_jump_open_threads() {
    let mut state = loaded_state(sample_sidebar());
    let effects = keys(&mut state, "] b");
    assert_eq!(effects[0], Effect::WatchThread(Some("thr_unread".into())));
    keys(&mut state, "2 ] b");
    assert_eq!(state.open_thread.as_deref(), Some("thr_other"));
    keys(&mut state, "SPC b n");
    assert_eq!(state.open_thread.as_deref(), Some("thr_wait"));
    keys(&mut state, "[ b");
    assert_eq!(state.open_thread.as_deref(), Some("thr_other"));
    keys(&mut state, "SPC `");
    assert_eq!(state.open_thread.as_deref(), Some("thr_wait"));
    keys(&mut state, ": b n RET");
    assert_eq!(state.open_thread.as_deref(), Some("thr_unread"));
    keys(&mut state, ": b SPC p a r s e r RET");
    assert_eq!(state.open_thread.as_deref(), Some("thr_other"));
    press_mod(&mut state, KeyCode::Char('3'), KeyModifiers::ALT);
    assert_eq!(state.open_thread.as_deref(), Some("thr_old"));
    assert_eq!(selected_key(&state), thread_key("thr_old"));
    assert!(press_mod(&mut state, KeyCode::Char('9'), KeyModifiers::ALT).is_empty());
    keys(&mut state, "SPC TAB 1");
    assert_eq!(state.open_thread.as_deref(), Some("thr_wait"));
}

#[test]
fn help_lists_bindings_of_the_focused_buffer_and_closes() {
    let mut state = loaded_state(sample_sidebar());
    keys(&mut state, "SPC h b b");
    let pane = state.result_pane.as_ref().expect("bindings pane");
    assert_eq!(pane.title, "Bindings: sidebar");
    let text = pane.copy_text.clone().unwrap_or_default();
    let has_row = |key: &str, desc: &str| {
        text.lines()
            .any(|line| line.starts_with(key) && line.contains(desc))
    };
    assert!(has_row("R ", "Rename thread"), "{text}");
    assert!(has_row("d d", "Archive / unarchive thread"), "{text}");
    assert!(has_row("SPC b k", "Archive / unarchive thread"), "{text}");
    assert!(has_row("M-m", "Pick model"), "{text}");
    assert!(has_row("C-w v", "Open the thread in a new split"), "{text}");
    press(&mut state, KeyCode::Char('q'));
    assert!(!state.quit);
    assert!(state.result_pane.is_none());
    press(&mut state, KeyCode::F(1));
    assert!(state.result_pane.is_some());
    press(&mut state, KeyCode::Esc);
    assert!(state.result_pane.is_none());
}

#[test]
fn keybinding_overrides_rebind_commands() {
    let mut state = loaded_state(sample_sidebar());
    update(
        &mut state,
        Action::KeymapLoaded(Ok(vec![AppKeybindingOverride {
            command: "thread.rename".into(),
            shortcut: Some(AppShortcut {
                key: "F2".into(),
                ..AppShortcut::default()
            }),
            platform: None,
        }])),
    );
    assert!(press(&mut state, KeyCode::Char('r')).is_empty());
    assert_eq!(state.mode, Mode::Normal);
    press(&mut state, KeyCode::F(2));
    assert!(matches!(state.mode, Mode::Rename { .. }));
    press(&mut state, KeyCode::Esc);
    keys(&mut state, "SPC b R");
    assert!(matches!(state.mode, Mode::Rename { .. }));
    assert!(
        state
            .toasts
            .iter()
            .any(|toast| toast.message == "Applied 1 keybinding override(s)")
    );
}

#[test]
fn snapshot_updates_keep_selection_on_same_thread() {
    let mut state = loaded_state(sample_sidebar());
    press(&mut state, KeyCode::Char('j'));
    assert_eq!(selected_key(&state), thread_key("thr_unread"));
    let mut sidebar = sample_sidebar();
    sidebar.projects[0].threads[0].thread.latest_attention_at = NOW_MS;
    sidebar.projects[0].threads[0].thread.last_read_at = Some(NOW_MS);
    update(&mut state, Action::Snapshot(snapshot_with(sidebar)));
    assert_eq!(selected_key(&state), thread_key("thr_unread"));
    assert_eq!(
        state.rows[1].thread().map(|row| row.id.as_str()),
        Some("thr_old")
    );

    let mut sidebar = sample_sidebar();
    sidebar.projects[0].threads.remove(1);
    update(&mut state, Action::Snapshot(snapshot_with(sidebar)));
    assert_eq!(state.selected, 3);
    assert_eq!(selected_key(&state), Some(RowKey::Group("proj_b".into())));
}

#[test]
fn sidebar_scrolls_to_keep_selection_visible() {
    let mut sidebar = sample_sidebar();
    for n in 0..30 {
        sidebar.projects[1].threads.push(thread(
            &format!("thr_{n:02}"),
            "proj_b",
            &format!("Task {n:02}"),
            100 + n,
        ));
    }
    let mut state = loaded_state(sidebar);
    press_mod(&mut state, KeyCode::Char('G'), KeyModifiers::SHIFT);
    let lines = render_lines(&state, 80, 13);
    assert!(lines[9].contains("Task 29"), "{lines:?}");
    press(&mut state, KeyCode::Char('k'));
    let lines = render_lines(&state, 80, 13);
    assert!(lines[8].contains("Task 28"), "{lines:?}");
    keys(&mut state, "1 0 G");
    assert_eq!(state.selected, 9);
    keys(&mut state, "z t");
    render_lines(&state, 80, 13);
    assert_eq!(state.sidebar_offset.get(), 9);
    keys(&mut state, "L");
    assert_eq!(state.selected, 17);
    keys(&mut state, "z b");
    render_lines(&state, 80, 13);
    assert_eq!(state.sidebar_offset.get(), 9);
    keys(&mut state, "C-e");
    assert_eq!(state.sidebar_offset.get(), 10);
    assert_eq!(state.selected, 17);
    keys(&mut state, "g g");
    let lines = render_lines(&state, 80, 13);
    assert!(lines[1].contains("alpha"), "{lines:?}");
}

#[test]
fn read_only_mode_drops_mutations() {
    let mut state = loaded_state(sample_sidebar());
    state.read_only = true;
    press(&mut state, KeyCode::Char('j'));
    let effects = without_composer_loads(press(&mut state, KeyCode::Enter));
    assert_eq!(
        effects,
        vec![Effect::WatchThread(Some("thr_unread".into()))]
    );
    assert!(state.toasts.is_empty());
    assert!(keys(&mut state, "d d").is_empty());
    assert_eq!(state.toasts.len(), 1);
    assert!(
        state.toasts[0]
            .message
            .contains("Read-only mode: cannot archive")
    );
    assert!(screen(&state).contains("read-only"));
}
