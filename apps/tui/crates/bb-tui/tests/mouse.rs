mod support;

use bb_tui::app::{Action, AppState, Effect, Focus, Mode, update};
use bb_tui::sidebar::RowKey;
use bb_tui::timeline::Scroll;
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;
use support::*;

const WIDTH: u16 = 100;
const HEIGHT: u16 = 24;

fn mouse(state: &mut AppState, kind: MouseEventKind, column: u16, row: u16) -> Vec<Effect> {
    update(
        state,
        Action::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }),
    )
}

fn click(state: &mut AppState, column: u16, row: u16) -> Vec<Effect> {
    mouse(state, MouseEventKind::Down(MouseButton::Left), column, row)
}

fn locate(state: &AppState, text: &str) -> (u16, u16) {
    let lines = render_lines(state, WIDTH, HEIGHT);
    let row = lines
        .iter()
        .position(|line| line.contains(text))
        .unwrap_or_else(|| panic!("{text} not on screen:\n{}", lines.join("\n")));
    let column = lines[row]
        .chars()
        .collect::<String>()
        .find(text)
        .map(|byte| lines[row][..byte].chars().count())
        .unwrap_or(0);
    (column as u16, row as u16)
}

fn opened_with_long_timeline() -> AppState {
    let mut state = loaded_state(sample_sidebar());
    let rows = (0..40)
        .map(|index| {
            if index % 2 == 0 {
                user(
                    &format!("u{index}"),
                    index * 10 + 1,
                    &format!("question {index}"),
                )
            } else {
                assistant(
                    &format!("a{index}"),
                    index * 10 + 1,
                    &format!("answer {index}"),
                )
            }
        })
        .collect();
    let snapshot = with_timeline(&state.snapshot, rows, false);
    update(&mut state, Action::Snapshot(snapshot));
    state.selected = state
        .rows
        .iter()
        .position(|row| row.thread().is_some_and(|thread| thread.id == THREAD))
        .expect("thread row");
    press(&mut state, KeyCode::Enter);
    render_lines(&state, WIDTH, HEIGHT);
    state
}

#[test]
fn clicking_a_sidebar_thread_opens_it_and_a_group_toggles() {
    let mut state = loaded_state(sample_sidebar());
    let (column, row) = locate(&state, "Refactor parser");
    let effects = click(&mut state, column, row);
    assert_eq!(state.open_thread.as_deref(), Some("thr_other"));
    assert_eq!(state.focus, Focus::Main);
    assert!(effects.contains(&Effect::WatchThread(Some("thr_other".into()))));

    let (column, row) = locate(&state, "▾ beta");
    click(&mut state, column, row);
    assert_eq!(
        state.rows[state.selected].key(),
        RowKey::Group("proj_b".into())
    );
    assert!(state.collapsed.contains("proj_b"));
    assert_eq!(state.focus, Focus::Sidebar);
}

#[test]
fn wheel_over_the_sidebar_moves_the_selection() {
    let mut state = loaded_state(sample_sidebar());
    let (column, row) = locate(&state, "alpha");
    let before = state.selected;
    mouse(&mut state, MouseEventKind::ScrollDown, column, row);
    assert_eq!(state.selected, (before + 3).min(state.rows.len() - 1));
    mouse(&mut state, MouseEventKind::ScrollUp, column, row);
    assert_eq!(state.selected, before);
}

#[test]
fn wheel_scrolls_the_timeline_and_click_selects_a_row() {
    let mut state = opened_with_long_timeline();
    let (column, row) = locate(&state, "answer 39");
    assert_eq!(state.timelines[THREAD].scroll, Scroll::Bottom);
    mouse(&mut state, MouseEventKind::ScrollUp, column, row);
    assert!(matches!(
        state.timelines[THREAD].scroll,
        Scroll::Anchor { .. }
    ));
    mouse(&mut state, MouseEventKind::ScrollDown, column, row);
    mouse(&mut state, MouseEventKind::ScrollDown, column, row);
    assert_eq!(state.timelines[THREAD].scroll, Scroll::Bottom);

    state.focus = Focus::Sidebar;
    let (column, row) = locate(&state, "question 38");
    click(&mut state, column, row);
    assert_eq!(state.focus, Focus::Main);
    let timeline = &state.timelines[THREAD];
    let selected = timeline.selected.as_deref().expect("selected row");
    assert!(selected.ends_with("u38"), "{selected}");
}

#[test]
fn clicking_the_composer_focuses_it() {
    let mut state = loaded_state(sample_sidebar());
    let (column, row) = locate(&state, "Refactor parser");
    click(&mut state, column, row);
    render_lines(&state, WIDTH, HEIGHT);
    let composer = state
        .mouse_areas
        .panes
        .borrow()
        .iter()
        .find_map(|pane| pane.composer)
        .expect("composer area");
    click(&mut state, composer.x + 2, composer.y + 1);
    assert_eq!(state.focus, Focus::Composer);
}

#[test]
fn overlays_take_the_wheel_and_ignore_clicks() {
    let mut state = loaded_state(sample_sidebar());
    render_lines(&state, WIDTH, HEIGHT);
    press(&mut state, KeyCode::Char('?'));
    assert_eq!(state.mode, Mode::Help);
    mouse(&mut state, MouseEventKind::ScrollDown, 5, 5);
    assert_eq!(state.mode, Mode::Help);
    press(&mut state, KeyCode::Esc);

    let selected = state.selected;
    press_mod(&mut state, KeyCode::Char('k'), KeyModifiers::CONTROL);
    assert!(state.palette.is_some());
    let (column, row) = (5, 3);
    click(&mut state, column, row);
    assert!(state.palette.is_some());
    assert_eq!(state.selected, selected);
}

#[test]
fn clicking_another_split_pane_focuses_it() {
    let mut state = loaded_state(sample_sidebar());
    state.open("thr_old".into());
    state.focus = Focus::Main;
    state.selected = state
        .rows
        .iter()
        .position(|row| row.thread().is_some_and(|thread| thread.id == "thr_other"))
        .expect("thread row");
    press(&mut state, KeyCode::Char('V'));
    assert_eq!(state.open_thread.as_deref(), Some("thr_other"));
    let (column, row) = locate(&state, "1 · Write docs");
    click(&mut state, column, row + 3);
    assert_eq!(state.open_thread.as_deref(), Some("thr_old"));
    assert_eq!(state.focus, Focus::Main);
}

#[test]
fn no_color_renders_without_colors() {
    let mut state = loaded_state(sample_sidebar());
    state.theme.no_color = true;
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("terminal");
    terminal
        .draw(|frame| bb_tui::ui::render(&state, frame))
        .expect("draw");
    let buffer = terminal.backend().buffer();
    assert!(
        buffer
            .content
            .iter()
            .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
    );
    assert!(
        buffer
            .content
            .iter()
            .any(|cell| cell.modifier.contains(ratatui::style::Modifier::REVERSED))
    );
}
