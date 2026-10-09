mod support;

use bb_tui::api::Mutation;
use bb_tui::app::{Action, AppState, Effect, Focus, Mode, update};
use bb_tui::evil::{Tag, tag};
use bb_tui::rbuf::{self, Buf};
use bb_wire::timeline::TimelineRow;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;
use support::*;

const WIDTH: u16 = 100;
const HEIGHT: u16 = 28;

fn opened(rows: Vec<TimelineRow>) -> AppState {
    let mut state = loaded_state(sample_sidebar());
    let snapshot = with_timeline(&state.snapshot, rows, false);
    update(&mut state, Action::Snapshot(snapshot));
    state.selected = state
        .rows
        .iter()
        .position(|row| row.thread().is_some_and(|thread| thread.id == THREAD))
        .expect("thread row");
    keys(&mut state, "g o");
    render_lines(&state, WIDTH, HEIGHT);
    state
}

fn conversation() -> AppState {
    opened(vec![
        user("u1", 1, "hello there"),
        assistant("a1", 2, "alpha one\n\nbeta two\n\ngamma three"),
        user("u2", 3, "closing words"),
    ])
}

fn cursor_text(state: &AppState, buf: Buf) -> String {
    let doc = rbuf::doc(state, buf);
    let (line, _) = rbuf::cursor(state, buf, &doc);
    doc.lines[line].clone()
}

fn cursor_col(state: &AppState, buf: Buf) -> usize {
    let doc = rbuf::doc(state, buf);
    rbuf::cursor(state, buf, &doc).1
}

fn type_search(state: &mut AppState, text: &str) {
    for ch in text.chars() {
        press(state, KeyCode::Char(ch));
    }
}

fn copied(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Copy(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn timeline_cursor_moves_over_rendered_lines_and_columns() {
    let mut state = conversation();
    keys(&mut state, "g g");
    assert!(cursor_text(&state, Buf::Timeline).contains("hello there"));
    keys(&mut state, "/ b e t a RET");
    assert!(cursor_text(&state, Buf::Timeline).contains("beta two"));
    let selected = state.timeline().unwrap().selected.clone();
    assert_eq!(selected.as_deref(), Some("a1"));
    keys(&mut state, "j j");
    assert!(cursor_text(&state, Buf::Timeline).contains("gamma three"));
    assert_eq!(
        state.timeline().unwrap().selected.as_deref(),
        Some("a1"),
        "moving inside a row keeps the row selected"
    );
    keys(&mut state, "^ w");
    let line = cursor_text(&state, Buf::Timeline);
    let col = cursor_col(&state, Buf::Timeline);
    assert_eq!(&line.chars().skip(col).collect::<String>(), "three");
    assert_eq!(copied(&keys(&mut state, "y i w")), vec!["three".to_owned()]);
    assert_eq!(
        state
            .registers
            .get(Some('0'))
            .map(|value| value.text.clone()),
        Some("three".into())
    );
    keys(&mut state, "}");
    assert_eq!(state.timeline().unwrap().selected.as_deref(), Some("u2"));
    keys(&mut state, "G");
    assert!(cursor_text(&state, Buf::Timeline).contains("closing words"));
    assert!(state.timeline().unwrap().cursor_follow);
}

#[test]
fn incremental_search_highlights_wraps_and_clears() {
    let mut state = conversation();
    keys(&mut state, "g g");
    press(&mut state, KeyCode::Char('/'));
    assert!(matches!(state.mode, Mode::Search(_)));
    assert_eq!(tag(&state), Tag::Command);
    type_search(&mut state, "gam");
    assert!(
        cursor_text(&state, Buf::Timeline).contains("gamma"),
        "the cursor follows the pattern while typing"
    );
    press(&mut state, KeyCode::Esc);
    assert!(cursor_text(&state, Buf::Timeline).contains("hello"));
    assert!(state.views.search.is_none());

    keys(&mut state, "/ o RET");
    assert!(
        state
            .views
            .search
            .as_ref()
            .is_some_and(|search| search.highlight)
    );
    let echo = state.keys.echo.clone().expect("echo");
    assert!(echo.text.starts_with("/o ["), "{}", echo.text);
    let first = cursor_text(&state, Buf::Timeline);
    keys(&mut state, "n");
    let second = cursor_text(&state, Buf::Timeline);
    assert_ne!(first, second);
    keys(&mut state, "G");
    let wrapped = (0..10).any(|_| {
        keys(&mut state, "N");
        state
            .keys
            .echo
            .as_ref()
            .is_some_and(|echo| echo.text.contains("search hit TOP"))
    });
    assert!(wrapped, "N wraps from the top to the bottom");

    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap();
    terminal
        .draw(|frame| bb_tui::ui::render(&state, frame))
        .unwrap();
    let highlighted = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .filter(|cell| cell.bg == Color::Yellow)
        .count();
    assert!(highlighted > 0, "matches are highlighted");

    keys(&mut state, ": n o h RET");
    assert!(
        state
            .views
            .search
            .as_ref()
            .is_some_and(|search| !search.highlight)
    );
    keys(&mut state, "n");
    assert!(
        state
            .views
            .search
            .as_ref()
            .is_some_and(|search| search.highlight)
    );
    press(&mut state, KeyCode::Esc);
    assert!(
        state
            .views
            .search
            .as_ref()
            .is_some_and(|search| !search.highlight)
    );
    keys(&mut state, "/ z z z RET");
    let echo = state.keys.echo.clone().expect("echo");
    assert!(echo.text.starts_with("E486"), "{}", echo.text);
}

#[test]
fn visual_selection_yanks_rendered_text() {
    let mut state = conversation();
    keys(&mut state, "/ a l p h a RET");
    keys(&mut state, "v");
    assert_eq!(tag(&state), Tag::Visual);
    keys(&mut state, "e");
    let effects = keys(&mut state, "y");
    assert_eq!(copied(&effects), vec!["alpha".to_owned()]);
    assert_eq!(tag(&state), Tag::Normal);
    assert!(state.views.visual.is_none());

    keys(&mut state, "V j j");
    assert_eq!(tag(&state), Tag::VisualLine);
    let effects = keys(&mut state, "y");
    let text = copied(&effects).pop().expect("copy");
    assert!(
        text.contains("alpha one") && text.contains("beta two"),
        "{text}"
    );
    assert!(text.ends_with('\n'));
    assert!(state.registers.get(None).unwrap().linewise);

    keys(&mut state, "g v");
    assert_eq!(tag(&state), Tag::VisualLine);
    press(&mut state, KeyCode::Esc);
    assert_eq!(tag(&state), Tag::Normal);

    keys(&mut state, "y");
    assert_eq!(tag(&state), Tag::Operator);
    press(&mut state, KeyCode::Esc);
    assert_eq!(tag(&state), Tag::Normal);
}

#[test]
fn code_block_and_row_objects_yank_their_text() {
    let mut state = opened(vec![assistant(
        "a1",
        1,
        "Intro text\n\n```rust\nfn parse() -> u8 { 3 }\nfn other() {}\n```\n\nOutro",
    )]);
    keys(&mut state, "g g / p a r s e RET");
    assert_eq!(
        copied(&keys(&mut state, "y i `")),
        vec!["fn parse() -> u8 { 3 }\nfn other() {}\n".to_owned()]
    );
    assert_eq!(
        copied(&keys(&mut state, "y a `")),
        vec!["```\nfn parse() -> u8 { 3 }\nfn other() {}\n```\n".to_owned()]
    );
    let row = copied(&keys(&mut state, "y i m")).pop().expect("row");
    assert!(row.contains("Intro text") && row.contains("Outro"), "{row}");
}

#[test]
fn marks_and_the_jump_list_return_to_earlier_lines() {
    let mut state = opened(long_rows(40));
    keys(&mut state, "g g");
    let top = cursor_text(&state, Buf::Timeline);
    keys(&mut state, "m a G");
    let bottom = cursor_text(&state, Buf::Timeline);
    assert_ne!(top, bottom);
    keys(&mut state, "' a");
    assert_eq!(cursor_text(&state, Buf::Timeline), top);
    keys(&mut state, "` `");
    assert_eq!(cursor_text(&state, Buf::Timeline), bottom);
    keys(&mut state, "g g");
    press_mod(&mut state, KeyCode::Char('o'), KeyModifiers::CONTROL);
    assert_eq!(cursor_text(&state, Buf::Timeline), bottom);
    press_mod(&mut state, KeyCode::Char('i'), KeyModifiers::CONTROL);
    assert_eq!(cursor_text(&state, Buf::Timeline), top);
    keys(&mut state, "' b");
    let echo = state.keys.echo.clone().expect("echo");
    assert_eq!(echo.text, "E20: Mark not set");
    keys(&mut state, "m A");
    keys(&mut state, ": m a r k s RET");
    let pane = state.result_pane.as_ref().expect("marks pane");
    assert_eq!(pane.title, "Marks");
}

#[test]
fn half_pages_and_screen_lines_move_the_cursor_with_the_view() {
    let mut state = opened(long_rows(60));
    keys(&mut state, "g g");
    let (top, height) = rbuf::view(&state, Buf::Timeline);
    assert_eq!(top, 0);
    press_mod(&mut state, KeyCode::Char('d'), KeyModifiers::CONTROL);
    let (after, _) = rbuf::view(&state, Buf::Timeline);
    assert_eq!(after, height / 2);
    keys(&mut state, "L");
    let doc = rbuf::doc(&state, Buf::Timeline);
    assert_eq!(
        rbuf::cursor(&state, Buf::Timeline, &doc).0,
        after + height - 1
    );
    keys(&mut state, "z t");
    assert_eq!(rbuf::view(&state, Buf::Timeline).0, after + height - 1);
    keys(&mut state, "H");
    let doc = rbuf::doc(&state, Buf::Timeline);
    assert_eq!(
        rbuf::cursor(&state, Buf::Timeline, &doc).0,
        rbuf::view(&state, Buf::Timeline).0
    );
}

fn long_rows(count: i64) -> Vec<TimelineRow> {
    (0..count)
        .map(|index| {
            if index % 2 == 0 {
                user(
                    &format!("u{index}"),
                    index + 10,
                    &format!("Question {index}"),
                )
            } else {
                assistant(&format!("a{index}"), index + 10, &format!("Answer {index}"))
            }
        })
        .collect()
}

fn sidebar_titles(state: &AppState) -> Vec<String> {
    state.rows.iter().map(rbuf::sidebar_label).collect()
}

#[test]
fn sidebar_search_reveals_collapsed_groups() {
    let mut state = loaded_state(sample_sidebar());
    assert_eq!(state.focus, Focus::Sidebar);
    let beta = sidebar_titles(&state)
        .iter()
        .position(|title| title == "beta")
        .expect("beta group");
    state.selected = beta;
    keys(&mut state, "z c");
    assert!(!sidebar_titles(&state).contains(&"Refactor parser".to_owned()));
    keys(&mut state, "g g / p a r s e r RET");
    assert_eq!(
        state.selected_thread_id(),
        Some("thr_other"),
        "{:?}",
        sidebar_titles(&state)
    );
    keys(&mut state, "g g * ");
    assert_eq!(state.selected, 0);
}

#[test]
fn sidebar_visual_line_batches_archive_and_read() {
    let mut state = loaded_state(sample_sidebar());
    keys(&mut state, "g g j V j");
    assert_eq!(tag(&state), Tag::VisualLine);
    let effects = keys(&mut state, "d");
    let archived: Vec<&str> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Mutate(Mutation::Archive { thread_id }) => Some(thread_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(archived.len(), 2, "{effects:?}");
    assert_eq!(tag(&state), Tag::Normal);
    let effects = keys(&mut state, "V j !");
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::Mutate(Mutation::MarkRead { .. }))),
        "{effects:?}"
    );
    let effects = keys(&mut state, "g g j V j y");
    let text = copied(&effects).pop().expect("titles");
    assert_eq!(text.lines().count(), 2, "{text}");
}

#[test]
fn output_pane_is_a_searchable_line_buffer() {
    let mut state = loaded_state(sample_sidebar());
    press(&mut state, KeyCode::F(1));
    render_lines(&state, WIDTH, HEIGHT);
    assert!(state.result_pane.is_some());
    keys(&mut state, "/ R e n a m e RET");
    let line = cursor_text(&state, Buf::Pane);
    assert!(line.contains("Rename"), "{line}");
    let effects = keys(&mut state, "y y");
    assert_eq!(copied(&effects), vec![format!("{line}\n")]);
    keys(&mut state, "j");
    assert!(state.result_pane.is_some(), "motions keep the pane open");
    keys(&mut state, "q");
    assert!(state.result_pane.is_none());
}

#[test]
fn register_prefix_names_the_yank_target_in_read_only_buffers() {
    let mut state = conversation();
    keys(&mut state, "/ a l p h a RET");
    keys(&mut state, "\" a y y");
    assert_eq!(state.focus, Focus::Main);
    let named = state
        .registers
        .get(Some('a'))
        .expect("register a")
        .text
        .clone();
    assert!(named.contains("alpha one"), "{named}");
    keys(&mut state, "\" _ y y");
    assert_eq!(state.registers.get(Some('a')).unwrap().text, named);
}
