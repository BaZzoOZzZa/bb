mod support;

use std::time::Duration;

use bb_tui::api::Mutation;
use bb_tui::app::{Action, AppState, Effect, Focus, Mode, update};
use bb_tui::composer::EditMode;
use bb_tui::evil::{Tag, tag};
use bb_wire::system::{AppKeybindingOverride, AppShortcut};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use support::*;

const THREAD: &str = "thr_other";

fn composing() -> AppState {
    let mut state = loaded_state(sample_sidebar());
    state.selected = state
        .rows
        .iter()
        .position(|row| row.thread().is_some_and(|thread| thread.id == THREAD))
        .expect("thread row");
    keys(&mut state, "g o i");
    assert_eq!(state.focus, Focus::Composer);
    state
}

fn write(state: &mut AppState, text: &str) {
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            update(
                state,
                Action::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL)),
            );
        }
        type_chars(state, line);
    }
}

fn text(state: &AppState) -> String {
    state.composer().expect("composer").text()
}

fn cursor(state: &AppState) -> usize {
    state.composer().expect("composer").cursor()
}

fn mode(state: &AppState) -> EditMode {
    state.composer().expect("composer").mode
}

fn echo(state: &AppState) -> Option<String> {
    state.keys.echo.as_ref().map(|echo| echo.text.clone())
}

fn tick(state: &mut AppState, after: Duration) {
    let now = state.clock + after;
    update(
        state,
        Action::Tick {
            now,
            now_ms: NOW_MS,
        },
    );
}

#[test]
fn escape_leaves_insert_and_normal_motions_move_over_words() {
    let mut state = composing();
    assert_eq!(tag(&state), Tag::Insert);
    write(&mut state, "foo.bar baz");
    press(&mut state, KeyCode::Esc);
    assert_eq!(mode(&state), EditMode::Normal);
    assert_eq!(tag(&state), Tag::Normal);
    assert_eq!(cursor(&state), 10);
    keys(&mut state, "0");
    assert_eq!(cursor(&state), 0);
    keys(&mut state, "w");
    assert_eq!(cursor(&state), 3);
    keys(&mut state, "W");
    assert_eq!(cursor(&state), 8);
    keys(&mut state, "b b");
    assert_eq!(cursor(&state), 3);
    keys(&mut state, "$");
    assert_eq!(cursor(&state), 10);
    keys(&mut state, "F o");
    assert_eq!(cursor(&state), 2);
    keys(&mut state, "^ 2 l");
    assert_eq!(cursor(&state), 2);
    keys(&mut state, "t a");
    assert_eq!(cursor(&state), 4);
    keys(&mut state, ";");
    assert_eq!(cursor(&state), 8);
    let lines = render_lines(&state, 140, 30);
    let status = &lines[lines.len() - 2];
    assert!(status.trim_end().ends_with("<N>"), "{status}");
    assert!(status.contains("[message]"), "{status}");
}

#[test]
fn operators_take_motions_text_objects_and_counts_with_undo() {
    let mut state = composing();
    write(&mut state, "one two three four");
    keys(&mut state, "ESC 0 d w");
    assert_eq!(text(&state), "two three four");
    keys(&mut state, "2 d w");
    assert_eq!(text(&state), "four");
    keys(&mut state, "u");
    assert_eq!(text(&state), "two three four");
    keys(&mut state, "u");
    assert_eq!(text(&state), "one two three four");
    keys(&mut state, "C-r");
    assert_eq!(text(&state), "two three four");
    keys(&mut state, "w c i w");
    assert_eq!(mode(&state), EditMode::Insert);
    type_chars(&mut state, "3");
    keys(&mut state, "ESC");
    assert_eq!(text(&state), "two 3 four");
    keys(&mut state, "u");
    assert_eq!(text(&state), "two three four");
    keys(&mut state, "0 3 x");
    assert_eq!(text(&state), " three four");
    keys(&mut state, "g U i w");
    assert_eq!(text(&state), " three four");
    keys(&mut state, "w g U i w");
    assert_eq!(text(&state), " THREE four");
    keys(&mut state, "~");
    assert_eq!(text(&state), " tHREE four");
    keys(&mut state, "D");
    assert_eq!(text(&state), " t");
    keys(&mut state, "u u");
    assert_eq!(text(&state), " THREE four");
}

#[test]
fn yank_put_registers_and_linewise_operations() {
    let mut state = composing();
    write(&mut state, "alpha\nbeta\ngamma");
    keys(&mut state, "ESC g g y y");
    assert_eq!(
        state.registers.get(None).map(|value| value.text.as_str()),
        Some("alpha\n")
    );
    assert_eq!(
        state.registers.get(Some('0')).map(|value| value.linewise),
        Some(true)
    );
    keys(&mut state, "j p");
    assert_eq!(text(&state), "alpha\nbeta\nalpha\ngamma");
    keys(&mut state, "d d");
    assert_eq!(text(&state), "alpha\nbeta\ngamma");
    assert_eq!(
        state
            .registers
            .get(Some('1'))
            .map(|value| value.text.as_str()),
        Some("alpha\n")
    );
    keys(&mut state, "\" a y i w");
    assert_eq!(
        state
            .registers
            .get(Some('a'))
            .map(|value| value.text.as_str()),
        Some("gamma")
    );
    keys(&mut state, "g g \" a P");
    assert_eq!(text(&state), "gammaalpha\nbeta\ngamma");
    keys(&mut state, "u g g J");
    assert_eq!(text(&state), "alpha beta\ngamma");
    keys(&mut state, "> >");
    assert_eq!(text(&state), "  alpha beta\ngamma");
    keys(&mut state, "< <");
    assert_eq!(text(&state), "alpha beta\ngamma");
    let effects = keys(&mut state, "y $");
    assert_eq!(effects, vec![Effect::Copy("alpha beta".into())]);
    let effects = keys(&mut state, "\" _ y y");
    assert!(effects.is_empty());
    keys(&mut state, "G \" x d d");
    assert_eq!(text(&state), "alpha beta");
    assert_eq!(
        state
            .registers
            .get(Some('x'))
            .map(|value| value.text.as_str()),
        Some("gamma\n")
    );
}

#[test]
fn visual_state_selects_and_operates() {
    let mut state = composing();
    write(&mut state, "keep this text");
    keys(&mut state, "ESC 0 w v");
    assert!(matches!(mode(&state), EditMode::Visual { line: false, .. }));
    assert_eq!(tag(&state), Tag::Visual);
    keys(&mut state, "e");
    let effects = keys(&mut state, "y");
    assert_eq!(effects, vec![Effect::Copy("this".into())]);
    assert_eq!(mode(&state), EditMode::Normal);
    keys(&mut state, "v i w U");
    assert_eq!(text(&state), "keep THIS text");
    keys(&mut state, "V");
    assert_eq!(tag(&state), Tag::VisualLine);
    keys(&mut state, "ESC");
    assert_eq!(mode(&state), EditMode::Normal);
    keys(&mut state, "w v $ d");
    assert_eq!(text(&state), "keep THIS ");
    keys(&mut state, "0 v e c");
    assert_eq!(mode(&state), EditMode::Insert);
    type_chars(&mut state, "drop");
    assert_eq!(text(&state), "drop THIS ");
}

#[test]
fn insert_keys_open_lines_and_one_normal_command() {
    let mut state = composing();
    write(&mut state, "middle");
    keys(&mut state, "ESC o");
    type_chars(&mut state, "below");
    keys(&mut state, "ESC g g O");
    type_chars(&mut state, "above");
    assert_eq!(text(&state), "above\nmiddle\nbelow");
    keys(&mut state, "C-o");
    assert_eq!(mode(&state), EditMode::Normal);
    keys(&mut state, "d d");
    assert_eq!(mode(&state), EditMode::Insert);
    assert_eq!(text(&state), "middle\nbelow");
    keys(&mut state, "ESC A C-w");
    assert_eq!(text(&state), "\nbelow");
    keys(&mut state, "ESC");
    keys(&mut state, "A");
    type_chars(&mut state, "x");
    keys(&mut state, "ESC");
    keys(&mut state, "j I");
    type_chars(&mut state, "<");
    assert_eq!(text(&state), "x\n<below");
    keys(&mut state, "ESC k g i");
    type_chars(&mut state, ">");
    assert_eq!(text(&state), "x\n<>below");
}

#[test]
fn leader_waits_for_which_key_and_reports_undefined_keys() {
    let mut state = loaded_state(sample_sidebar());
    keys(&mut state, "SPC");
    assert!(state.keys.is_pending());
    assert!(!state.keys.which_key);
    tick(&mut state, Duration::ZERO);
    tick(&mut state, Duration::from_millis(100));
    assert!(!state.keys.which_key);
    tick(&mut state, Duration::from_millis(300));
    assert!(state.keys.which_key);
    let screen = render_lines(&state, 140, 40).join("\n");
    assert!(screen.contains("SPC-"), "{screen}");
    assert!(screen.contains("b → +buffer"), "{screen}");
    assert!(screen.contains(": → Command palette"), "{screen}");
    keys(&mut state, "b");
    let screen = render_lines(&state, 140, 40).join("\n");
    assert!(screen.contains("SPC b-"), "{screen}");
    assert!(
        screen.contains("k → Archive / unarchive thread"),
        "{screen}"
    );
    assert_eq!(state.keys.showcmd(), "SPC b");
    keys(&mut state, "Bksp");
    assert_eq!(state.keys.showcmd(), "SPC");
    let effects = keys(&mut state, "b z");
    assert!(effects.is_empty());
    assert!(!state.keys.is_pending());
    assert_eq!(echo(&state).as_deref(), Some("SPC b z is undefined"));
    keys(&mut state, "SPC ESC");
    assert!(!state.keys.is_pending());
}

#[test]
fn which_key_pages_when_the_window_is_small() {
    let mut state = loaded_state(sample_sidebar());
    keys(&mut state, "SPC");
    tick(&mut state, Duration::ZERO);
    tick(&mut state, Duration::from_secs(1));
    let screen = render_lines(&state, 60, 12).join("\n");
    assert!(screen.contains("page 1/"), "{screen}");
    keys(&mut state, "C-n");
    assert_eq!(state.keys.which_key_page, 1);
    let screen = render_lines(&state, 60, 12).join("\n");
    assert!(screen.contains("page 2/"), "{screen}");
    assert!(state.keys.is_pending());
}

#[test]
fn describe_key_explains_instead_of_running() {
    let mut state = loaded_state(sample_sidebar());
    keys(&mut state, "SPC h k");
    let effects = keys(&mut state, "SPC b k");
    assert!(effects.is_empty());
    assert_eq!(
        echo(&state).as_deref(),
        Some("SPC b k runs Archive / unarchive thread")
    );
    let effects = keys(&mut state, "SPC b k");
    assert_eq!(
        effects,
        vec![Effect::Mutate(Mutation::Archive {
            thread_id: "thr_wait".into()
        })]
    );
}

#[test]
fn server_overrides_shadow_evil_keys_but_not_insert_typing() {
    let mut state = composing();
    update(
        &mut state,
        Action::KeymapLoaded(Ok(vec![
            AppKeybindingOverride {
                command: "thread.archive".into(),
                shortcut: Some(AppShortcut {
                    key: "j".into(),
                    ..AppShortcut::default()
                }),
                platform: None,
            },
            AppKeybindingOverride {
                command: "file.quickOpen".into(),
                shortcut: Some(AppShortcut {
                    key: "y".into(),
                    primary_modifier: true,
                    ..AppShortcut::default()
                }),
                platform: None,
            },
            AppKeybindingOverride {
                command: "palette.open".into(),
                shortcut: Some(AppShortcut {
                    key: "w".into(),
                    primary_modifier: true,
                    ..AppShortcut::default()
                }),
                platform: None,
            },
        ])),
    );
    let effects = type_chars(&mut state, "j");
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, Effect::Mutate(_)))
    );
    assert_eq!(text(&state), "j");
    keys(&mut state, "C-w");
    assert!(state.palette.is_none());
    assert_eq!(text(&state), "");
    keys(&mut state, "C-y");
    assert!(state.palette.is_some());
    keys(&mut state, "ESC");
    keys(&mut state, "ESC C-k");
    assert_eq!(state.focus, Focus::Main);
    let effects = keys(&mut state, "j");
    assert_eq!(
        effects,
        vec![Effect::Mutate(Mutation::Archive {
            thread_id: THREAD.into()
        })]
    );
    keys(&mut state, "C-w");
    assert!(state.palette.is_some());
}

#[test]
fn meta_space_reaches_the_leader_from_insert_and_ctrl_c_sequences_send() {
    let mut state = composing();
    write(&mut state, "draft");
    let effects = keys(&mut state, "M-SPC b k");
    assert_eq!(
        effects,
        vec![Effect::Mutate(Mutation::Archive {
            thread_id: THREAD.into()
        })]
    );
    assert_eq!(text(&state), "draft");
    assert_eq!(mode(&state), EditMode::Insert);
    keys(&mut state, "C-c");
    assert_eq!(state.keys.showcmd(), "C-c");
    assert_eq!(tag(&state), Tag::Insert);
    keys(&mut state, "C-g");
    assert!(!state.keys.is_pending());
    assert_eq!(text(&state), "draft");
}

#[test]
fn ex_line_runs_commands_reports_errors_and_keeps_history() {
    let mut state = loaded_state(sample_sidebar());
    keys(&mut state, ":");
    assert!(matches!(state.mode, Mode::Ex(_)));
    assert_eq!(tag(&state), Tag::Command);
    type_chars(&mut state, "frobnicate");
    let lines = render_lines(&state, 100, 20);
    assert_eq!(
        lines.last().map(|line| line.trim_end()),
        Some(":frobnicate")
    );
    keys(&mut state, "RET");
    assert_eq!(state.mode, Mode::Normal);
    assert_eq!(
        echo(&state).as_deref(),
        Some("E492: Not an editor command: frobnicate")
    );
    keys(
        &mut state,
        ": s e t SPC w h i c h k e y d e l a y = 1 2 0 RET",
    );
    assert_eq!(state.keys.which_key_delay, Duration::from_millis(120));
    keys(&mut state, ": ↑");
    let Mode::Ex(line) = &state.mode else {
        panic!("ex line");
    };
    assert_eq!(line.input.text(), "set whichkeydelay=120");
    keys(&mut state, "C-u ESC");
    assert_eq!(state.mode, Mode::Normal);
    keys(&mut state, ": 4 RET");
    assert_eq!(state.selected, 3);
    keys(&mut state, ": s p RET");
    assert_eq!(state.open_thread.as_deref(), Some("thr_old"));
    assert_eq!(state.focus, Focus::Main);
    keys(&mut state, ": q a RET");
    assert!(state.quit);
}

#[test]
fn quit_needs_an_explicit_command() {
    let mut state = loaded_state(sample_sidebar());
    keys(&mut state, "q");
    assert!(!state.quit);
    keys(&mut state, "C-c");
    assert!(!state.quit);
    keys(&mut state, "ESC Z Q");
    assert!(!state.quit);
    assert!(!state.sidebar_visible);
    keys(&mut state, "Z Q");
    assert!(state.quit);

    let mut state = loaded_state(sample_sidebar());
    keys(&mut state, "SPC o p");
    assert!(!state.sidebar_visible);
    keys(&mut state, ": q RET");
    assert!(state.quit);
}

#[test]
fn russian_layout_keys_act_in_normal_state_only() {
    let mut state = loaded_state(sample_sidebar());
    let before = state.selected;
    press(&mut state, KeyCode::Char('о'));
    assert_eq!(state.selected, before + 1);
    let mut composer = composing();
    type_chars(&mut composer, "до");
    assert_eq!(text(&composer), "до");
    keys(&mut composer, "ESC");
    press(&mut composer, KeyCode::Char('ш'));
    assert_eq!(mode(&composer), EditMode::Insert);
}

#[test]
fn modeline_shows_pending_count_and_operator_state() {
    let mut state = composing();
    write(&mut state, "abc");
    keys(&mut state, "ESC 2 d");
    assert_eq!(state.keys.showcmd(), "2d");
    assert_eq!(tag(&state), Tag::Operator);
    let lines = render_lines(&state, 140, 30);
    let status = &lines[lines.len() - 2];
    assert!(status.contains("2d"), "{status}");
    assert!(status.trim_end().ends_with("<O>"), "{status}");
    keys(&mut state, "ESC");
    assert_eq!(tag(&state), Tag::Normal);
    assert_eq!(text(&state), "abc");
}
