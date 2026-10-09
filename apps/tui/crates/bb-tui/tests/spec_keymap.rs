mod support;

use bb_tui::app::{AppState, Focus, Mode};
use bb_tui::keymap::{Act, Command, KeyChord, Keymap, Resolved, Scope};
use bb_tui::palette::PaletteKind;
use support::*;

const SPEC: &str = include_str!("../../../docs/keymap-evil.md");

const NOT_BUILT: &[&str] = &["SPC f r", "SPC s i", "SPC t l", "SPC g o P"];

const SPECIAL: &[&str] = &["M-SPC", "M-SPC m", "C-[", "ESC", "C-c"];

fn section(number: &str) -> &'static str {
    let start = SPEC
        .find(&format!("\n## {number}. "))
        .unwrap_or_else(|| panic!("section {number}"));
    let rest = &SPEC[start + 1..];
    let end = rest[3..]
        .find("\n## ")
        .map_or(rest.len(), |index| index + 3);
    &rest[..end]
}

fn first_cells(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|line| line.starts_with("| `"))
        .filter_map(|line| line.split(" | ").next())
        .map(|cell| cell.trim_start_matches('|').trim())
        .collect()
}

fn tokens(cell: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = cell;
    while let Some(open) = rest.find('`') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('`') else {
            break;
        };
        found.push(after[..close].to_owned());
        rest = &after[close + 1..];
    }
    let prefix = found
        .first()
        .and_then(|first| first.rsplit_once(' ').map(|(head, _)| head.to_owned()));
    found
        .into_iter()
        .enumerate()
        .map(|(index, token)| match &prefix {
            Some(head) if index > 0 && !token.contains(' ') && token.chars().count() == 1 => {
                format!("{head} {token}")
            }
            _ => token,
        })
        .filter(|token| !token.contains('{') && !token.is_empty())
        .collect()
}

fn chords(token: &str) -> Vec<KeyChord> {
    let mut keys = Vec::new();
    for part in token.split(' ').filter(|part| !part.is_empty()) {
        match KeyChord::parse(part) {
            Some(chord) => keys.push(chord),
            None => keys.extend(part.chars().map(KeyChord::ch)),
        }
    }
    keys
}

fn resolves(keymap: &Keymap, scopes: &[Scope], token: &str) -> bool {
    let keys = chords(token);
    if let [chord] = keys.as_slice()
        && keymap.lookup_chord(*chord).is_some()
    {
        return true;
    }
    for length in 1..keys.len() {
        if let Resolved::Exact(binding) = keymap.resolve(scopes, &keys[..length]) {
            return matches!(binding.act, Act::Char(_)) && length + 1 == keys.len();
        }
    }
    !matches!(keymap.resolve(scopes, &keys), Resolved::Unbound)
}

fn check(scopes: &[Scope], cells: Vec<&str>) -> Vec<String> {
    let keymap = Keymap::default();
    let mut missing = Vec::new();
    for cell in cells {
        for token in tokens(cell) {
            if NOT_BUILT.contains(&token.as_str()) || SPECIAL.contains(&token.as_str()) {
                continue;
            }
            if !resolves(&keymap, scopes, &token) {
                missing.push(token);
            }
        }
    }
    missing
}

#[test]
fn global_leader_and_window_keys_of_the_spec_are_bound() {
    let mut cells = first_cells(section("4"));
    cells.extend(first_cells(section("5")));
    cells.extend(first_cells(section("6")));
    let missing = check(&[Scope::Global], cells);
    assert!(missing.is_empty(), "unbound: {missing:?}");
}

#[test]
fn leader_entries_are_exact_bindings_not_dangling_prefixes() {
    let keymap = Keymap::default();
    let mut dangling = Vec::new();
    for cell in first_cells(section("5")) {
        for token in tokens(cell) {
            if !token.starts_with("SPC ") || NOT_BUILT.contains(&token.as_str()) {
                continue;
            }
            let keys = chords(&token);
            let group = keymap.group_name(&[Scope::Global], &keys).is_some();
            let exact = matches!(keymap.resolve(&[Scope::Global], &keys), Resolved::Exact(_));
            if !group && !exact {
                dangling.push(token);
            }
        }
    }
    assert!(dangling.is_empty(), "not exact: {dangling:?}");
}

#[test]
fn sidebar_keys_of_the_spec_are_bound() {
    let missing = check(&[Scope::Sidebar, Scope::Global], first_cells(section("8")));
    assert!(missing.is_empty(), "unbound: {missing:?}");
}

#[test]
fn timeline_keys_of_the_spec_are_bound() {
    let text = section("9");
    let (keys, objects) = text
        .split_once("Text objects usable")
        .expect("objects table");
    let mut missing = check(&[Scope::Timeline, Scope::Global], first_cells(keys));
    for cell in first_cells(objects) {
        for token in tokens(cell) {
            for kind in ["i", "a"] {
                let keys = format!("y {kind} {token}");
                if !resolves(&Keymap::default(), &[Scope::Timeline], &keys) {
                    missing.push(keys);
                }
            }
        }
    }
    assert!(missing.is_empty(), "unbound: {missing:?}");
}

#[test]
fn motion_scroll_and_fold_keys_of_the_spec_work_in_read_only_buffers() {
    let skip = [
        "'{mark}",
        "`{mark}",
        "''",
        "``",
        "/{pattern}RET",
        "?{pattern}RET",
    ];
    let text = section("7");
    let cells: Vec<&str> = first_cells(text)
        .into_iter()
        .filter(|cell| !skip.iter().any(|skip| cell.contains(skip)))
        .collect();
    let missing: Vec<String> = check(&[Scope::Timeline, Scope::Global], cells)
        .into_iter()
        .filter(|token| !["m{A-Z}", ":noh", ":marks", "z j", "zj", "zk"].contains(&token.as_str()))
        .filter(|token| !token.starts_with(':'))
        .collect();
    assert!(missing.is_empty(), "unbound: {missing:?}");
}

#[test]
fn composer_normal_visual_and_insert_keys_of_the_spec_are_bound() {
    let text = section("10");
    let normal = text
        .split("### Normal state")
        .nth(1)
        .and_then(|rest| rest.split("### Visual states").next())
        .expect("normal table");
    let missing = check(&[Scope::Composer, Scope::Global], first_cells(normal))
        .into_iter()
        .filter(|token| !token.starts_with(':'))
        .collect::<Vec<_>>();
    assert!(missing.is_empty(), "normal unbound: {missing:?}");
    let visual = text
        .split("### Visual states")
        .nth(1)
        .and_then(|rest| rest.split("### The new-thread form").next())
        .expect("visual table");
    let missing = check(
        &[Scope::Visual, Scope::Composer, Scope::Global],
        first_cells(visual),
    );
    assert!(missing.is_empty(), "visual unbound: {missing:?}");
}

#[test]
fn every_command_is_reachable_by_a_key_or_an_ex_command() {
    let keymap = Keymap::default();
    let unreachable: Vec<Command> = Command::ALL
        .iter()
        .copied()
        .filter(|command| keymap.labels_for(*command).is_empty())
        .collect();
    assert!(unreachable.is_empty(), "unreachable: {unreachable:?}");
}

#[test]
fn right_panel_diff_and_terminal_keys_of_the_spec_are_bound() {
    let text = section("14");
    let table = |heading: &str, end: &str| -> Vec<&'static str> {
        let body = text
            .split(heading)
            .nth(1)
            .and_then(|rest| rest.split(end).next())
            .unwrap_or_else(|| panic!("table {heading}"));
        first_cells(body)
    };
    let cases = [
        (
            table("### Right panel", "Info tab:"),
            vec![Scope::PanelInfo, Scope::Panel, Scope::Global],
        ),
        (
            table("Diff tab:", "File tab:"),
            vec![Scope::PanelDiff, Scope::Panel, Scope::Global],
        ),
        (
            table("### Terminal tab", "### Notification center"),
            vec![Scope::PanelTerminal, Scope::Panel, Scope::Global],
        ),
    ];
    for (cells, scopes) in cases {
        let missing: Vec<String> = check(&scopes, cells)
            .into_iter()
            .filter(|token| !["C-\\", "C-\\ C-n", "S-PgUp", "S-PgDn"].contains(&token.as_str()))
            .collect();
        assert!(missing.is_empty(), "{scopes:?} unbound: {missing:?}");
    }
}

fn sidebar_on_thread() -> AppState {
    let mut state = loaded_state(sample_sidebar());
    state.selected = state
        .rows
        .iter()
        .position(|row| row.thread().is_some_and(|thread| thread.id == "thr_other"))
        .expect("thread row");
    state
}

fn palette(state: &AppState) -> Option<PaletteKind> {
    state.palette.as_ref().map(|palette| palette.kind)
}

fn ex_text(state: &AppState) -> Option<String> {
    match &state.mode {
        Mode::Ex(line) => Some(line.input.text().to_owned()),
        _ => None,
    }
}

type Check = fn(&AppState) -> bool;

#[test]
fn leader_and_global_sequences_drive_the_app() {
    let cases: &[(&str, Check)] = &[
        ("SPC b b", |s| palette(s) == Some(PaletteKind::Threads)),
        ("SPC ,", |s| palette(s) == Some(PaletteKind::Threads)),
        ("SPC /", |s| palette(s) == Some(PaletteKind::Threads)),
        ("SPC p p", |s| palette(s) == Some(PaletteKind::Projects)),
        ("SPC TAB TAB", |s| palette(s) == Some(PaletteKind::Projects)),
        ("SPC :", |s| palette(s) == Some(PaletteKind::Commands)),
        ("M-x", |s| palette(s) == Some(PaletteKind::Commands)),
        ("SPC ;", |s| ex_text(s).as_deref() == Some("")),
        (":", |s| ex_text(s).as_deref() == Some("")),
        ("SPC h x", |s| ex_text(s).as_deref() == Some("help :")),
        ("SPC o p", |s| !s.sidebar_visible),
        ("SPC t s", |s| !s.sidebar_visible),
        ("SPC o i", |s| s.panel_visible),
        ("SPC h ,", |s| s.settings.is_some()),
        ("SPC n n", |s| matches!(s.mode, Mode::Notifications { .. })),
        ("SPC o A", |s| s.automations.is_some()),
        ("SPC h b b", |s| s.result_pane.is_some()),
        ("F1", |s| s.result_pane.is_some()),
        ("SPC h ?", |s| s.result_pane.is_some()),
        ("SPC h e", |s| s.result_pane.is_some()),
        ("SPC s r", |s| s.result_pane.is_some()),
        ("SPC h k", |s| s.keys.describe),
        ("SPC t h", |s| !s.views.hlsearch),
        ("SPC q q", |s| s.quit),
        ("SPC q Q", |s| s.quit),
        ("Z Q", |s| !s.sidebar_visible && !s.quit),
        ("g o Z Q", |s| s.quit),
        ("q", |s| !s.quit),
        ("g o", |s| {
            s.focus == Focus::Main && s.open_thread.as_deref() == Some("thr_other")
        }),
        ("RET", |s| {
            s.focus == Focus::Sidebar && s.open_thread.as_deref() == Some("thr_other")
        }),
        ("i", |s| s.focus == Focus::Composer),
        ("C-w v", |s| s.open_thread.as_deref() == Some("thr_other")),
        ("g o C-w h", |s| s.focus == Focus::Sidebar),
        ("g o C-w j", |s| s.focus == Focus::Composer),
        ("g o C-j C-k", |s| s.focus == Focus::Main),
        ("g o C-w w", |s| s.focus == Focus::Composer),
        ("g o SPC a c", |s| s.focus == Focus::Composer),
        ("g o SPC s s", |s| matches!(s.mode, Mode::Search(_))),
        ("g o /", |s| matches!(s.mode, Mode::Search(_))),
    ];
    let mut failed = Vec::new();
    for (sequence, check) in cases {
        let mut state = sidebar_on_thread();
        keys(&mut state, sequence);
        if !check(&state) {
            failed.push(*sequence);
        }
    }
    assert!(failed.is_empty(), "sequences without effect: {failed:?}");
}
