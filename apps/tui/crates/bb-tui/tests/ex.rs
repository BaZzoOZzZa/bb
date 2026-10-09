mod support;

use bb_tui::app::{Action, AppState, Effect, Focus, Mode, update};
use bb_tui::palette::{PaletteLoaded, PaletteRequest};
use bb_wire::project::{WorkspacePathEntry, WorkspacePathListResponse};
use support::*;

fn echo(state: &AppState) -> Option<(String, bool)> {
    state
        .keys
        .echo
        .as_ref()
        .map(|echo| (echo.text.clone(), echo.error))
}

fn ex_text(state: &AppState) -> String {
    let Mode::Ex(line) = &state.mode else {
        panic!("ex line is closed: {:?}", state.mode);
    };
    line.input.text().to_owned()
}

fn wild_items(state: &AppState) -> Vec<String> {
    let Mode::Ex(line) = &state.mode else {
        panic!("ex line is closed");
    };
    line.wild
        .as_ref()
        .map(|wild| wild.items.clone())
        .unwrap_or_default()
}

fn ex(state: &mut AppState, text: &str) -> Vec<Effect> {
    keys(state, ":");
    let mut effects = type_chars(state, text);
    effects.extend(keys(state, "RET"));
    effects
}

#[test]
fn tab_completes_command_names_and_cycles_through_matches() {
    let mut state = loaded_state(sample_sidebar());
    keys(&mut state, ":");
    type_chars(&mut state, "se");
    keys(&mut state, "TAB");
    assert_eq!(ex_text(&state), "set");
    assert_eq!(wild_items(&state), ["set", "search", "settings"]);
    let screen = render_lines(&state, 100, 24).join("\n");
    assert!(screen.contains("completion"), "{screen}");
    assert!(screen.contains("settings"), "{screen}");
    keys(&mut state, "TAB");
    assert_eq!(ex_text(&state), "search");
    keys(&mut state, "S-TAB S-TAB");
    assert_eq!(ex_text(&state), "se");
    keys(&mut state, "C-p");
    assert_eq!(ex_text(&state), "settings");
    type_chars(&mut state, " ");
    assert!(wild_items(&state).is_empty());
    keys(&mut state, "TAB");
    assert_eq!(wild_items(&state)[0], "general");
    keys(&mut state, "ESC");
    assert_eq!(state.mode, Mode::Normal);

    keys(&mut state, ":");
    type_chars(&mut state, "sett");
    keys(&mut state, "TAB");
    assert_eq!(ex_text(&state), "settings");
}

#[test]
fn tab_completes_threads_options_and_help_topics() {
    let mut state = loaded_state(sample_sidebar());
    keys(&mut state, ":");
    type_chars(&mut state, "b doc");
    keys(&mut state, "TAB");
    assert_eq!(ex_text(&state), "b Write docs");
    keys(&mut state, "RET");
    assert_eq!(state.open_thread.as_deref(), Some("thr_old"));
    assert_eq!(state.focus, Focus::Main);

    keys(&mut state, ":");
    type_chars(&mut state, "set nohl");
    keys(&mut state, "TAB");
    assert_eq!(ex_text(&state), "set nohlsearch");
    type_chars(&mut state, " submitkey=c");
    keys(&mut state, "TAB");
    assert_eq!(ex_text(&state), "set nohlsearch submitkey=ctrl-enter");
    keys(&mut state, "ESC");

    keys(&mut state, ":");
    type_chars(&mut state, "help :bn");
    keys(&mut state, "TAB");
    assert_eq!(ex_text(&state), "help :bnext");
    keys(&mut state, "C-u");
    type_chars(&mut state, "frob");
    keys(&mut state, "TAB");
    let Mode::Ex(line) = &state.mode else {
        panic!("ex line");
    };
    assert_eq!(
        line.wild.as_ref().and_then(|wild| wild.note.as_deref()),
        Some("No match")
    );
}

#[test]
fn file_arguments_complete_from_the_workspace_search() {
    let mut state = loaded_state(sample_sidebar());
    keys(&mut state, ":");
    type_chars(&mut state, "e ma");
    let effects = keys(&mut state, "TAB");
    let Some((generation, request)) = effects.iter().find_map(|effect| match effect {
        Effect::Palette {
            generation,
            request,
        } => Some((*generation, request.clone())),
        _ => None,
    }) else {
        panic!("no path request in {effects:?}");
    };
    let PaletteRequest::Paths { query, .. } = request else {
        panic!("unexpected request {request:?}");
    };
    assert_eq!(query, "ma");
    let entry = |path: &str, score: f64| WorkspacePathEntry {
        path: path.into(),
        name: path.rsplit('/').next().unwrap_or(path).into(),
        score,
        ..WorkspacePathEntry::default()
    };
    update(
        &mut state,
        Action::PaletteLoaded {
            generation,
            loaded: PaletteLoaded::Paths {
                query: "ma".into(),
                result: Ok(WorkspacePathListResponse {
                    paths: vec![entry("docs/manual.md", 1.0), entry("src/main.rs", 5.0)],
                    ..WorkspacePathListResponse::default()
                }),
            },
        },
    );
    assert_eq!(ex_text(&state), "e src/main.rs");
    assert_eq!(wild_items(&state), ["src/main.rs", "docs/manual.md"]);
    keys(&mut state, "TAB");
    assert_eq!(ex_text(&state), "e docs/manual.md");
}

#[test]
fn history_walks_entries_that_start_with_the_typed_prefix() {
    let mut state = loaded_state(sample_sidebar());
    ex(&mut state, "set ws");
    ex(&mut state, "noh");
    ex(&mut state, "set hls");
    keys(&mut state, ":");
    type_chars(&mut state, "se");
    keys(&mut state, "↑");
    assert_eq!(ex_text(&state), "set hls");
    keys(&mut state, "C-p");
    assert_eq!(ex_text(&state), "set ws");
    keys(&mut state, "↑");
    assert_eq!(ex_text(&state), "set ws");
    keys(&mut state, "↓");
    assert_eq!(ex_text(&state), "set hls");
    keys(&mut state, "C-n");
    assert_eq!(ex_text(&state), "se");
    keys(&mut state, "C-u ↑");
    assert_eq!(ex_text(&state), "set hls");
}

#[test]
fn set_options_report_values_and_errors() {
    let mut state = loaded_state(sample_sidebar());
    ex(&mut state, "set ic?");
    assert_eq!(echo(&state), Some(("  ignorecase".into(), false)));
    ex(&mut state, "set noic scs");
    assert!(!state.views.case.ignorecase);
    assert!(state.views.case.smartcase);
    ex(&mut state, "set invic");
    assert!(state.views.case.ignorecase);
    ex(&mut state, "set nois");
    assert!(!state.views.incsearch);
    ex(&mut state, "set nohls");
    assert!(!state.views.hlsearch);
    ex(&mut state, "set whichkeydelay");
    assert_eq!(echo(&state), Some(("  whichkeydelay=300".into(), false)));
    ex(&mut state, "set rnu");
    assert_eq!(
        echo(&state),
        Some(("E519: Option not supported: rnu".into(), true))
    );
    ex(&mut state, "set frob");
    assert_eq!(
        echo(&state),
        Some(("E518: Unknown option: frob".into(), true))
    );
    ex(&mut state, "set whichkeydelay=soon");
    assert_eq!(
        echo(&state),
        Some((
            "E521: Number required after =: whichkeydelay=soon".into(),
            true
        ))
    );
    ex(&mut state, "set hlsearch=1");
    assert_eq!(
        echo(&state),
        Some(("E474: Invalid argument: hlsearch=1".into(), true))
    );
    ex(&mut state, "set");
    let pane = state.result_pane.as_ref().expect("options pane");
    let text = pane.copy_text.clone().unwrap_or_default();
    assert!(text.contains("noincsearch"), "{text}");
    assert!(text.contains("nohlsearch"), "{text}");
    assert!(!text.contains("wrapscan"), "{text}");
}

#[test]
fn help_opens_the_keymap_spec_at_the_topic() {
    let mut state = loaded_state(sample_sidebar());
    ex(&mut state, "help");
    let pane = state.result_pane.as_ref().expect("help pane");
    assert_eq!(pane.title, "Help: keymap-evil.md");
    assert_eq!(pane.cursor, 0);
    assert!(
        pane.copy_text
            .as_deref()
            .is_some_and(|text| text.starts_with("# bb-tui evil keymap"))
    );
    ex(&mut state, "h sidebar");
    let pane = state.result_pane.as_ref().expect("help pane");
    let text = pane.copy_text.clone().unwrap_or_default();
    let line = text.lines().nth(pane.cursor).unwrap_or_default();
    assert!(line.starts_with("## 8. Sidebar"), "{line}");
    assert_eq!(pane.scroll, pane.cursor);
    let screen = render_lines(&state, 120, 30).join("\n");
    assert!(screen.contains("## 8. Sidebar"), "{screen}");

    ex(&mut state, "h :wq");
    let pane = state.result_pane.as_ref().expect("help pane");
    let text = pane.copy_text.clone().unwrap_or_default();
    let line = text.lines().nth(pane.cursor).unwrap_or_default();
    assert!(line.contains("`:wq`"), "{line}");

    keys(&mut state, "q");
    assert!(state.result_pane.is_none());
    ex(&mut state, "help nosuchthingatall");
    assert_eq!(
        echo(&state),
        Some(("E149: Sorry, no help for nosuchthingatall".into(), true))
    );
    ex(&mut state, "messages");
    let pane = state.result_pane.as_ref().expect("messages pane");
    assert!(
        pane.copy_text
            .as_deref()
            .is_some_and(|text| text.contains("E149: Sorry, no help for nosuchthingatall"))
    );
}

#[test]
fn read_only_mode_refuses_mutating_ex_commands() {
    let mut state = loaded_state(sample_sidebar());
    state.read_only = true;
    let effects = ex(&mut state, "pin");
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(
        echo(&state),
        Some(("E45: Read-only mode: :pin would change data".into(), true))
    );
    let effects = ex(&mut state, "ren New title");
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(
        echo(&state),
        Some((
            "E45: Read-only mode: :rename would change data".into(),
            true
        ))
    );
    ex(&mut state, "set nows");
    assert!(!state.views.wrapscan);
}

#[test]
fn at_colon_repeats_the_last_ex_command() {
    let mut state = loaded_state(sample_sidebar());
    keys(&mut state, "@ :");
    assert_eq!(
        echo(&state),
        Some(("E30: No previous command line".into(), true))
    );
    ex(&mut state, "set invws");
    assert!(!state.views.wrapscan);
    keys(&mut state, "@ :");
    assert!(state.views.wrapscan);
    keys(&mut state, "3 @ :");
    assert!(!state.views.wrapscan);
}
