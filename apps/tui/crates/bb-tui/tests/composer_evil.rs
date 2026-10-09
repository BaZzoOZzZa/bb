mod support;

use bb_tui::app::{Action, AppState, Effect, Focus, update};
use bb_tui::chips::char_to_utf16;
use bb_tui::composer::EditMode;
use bb_tui::evil::{Tag, tag};
use bb_tui::prompt::PromptDraft;
use bb_wire::prompt::{
    PathEntryKind, PromptMentionPathSource, PromptMentionResource, PromptTextMention,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
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

fn with_text(text: &str) -> AppState {
    let mut state = composing();
    set_draft(&mut state, PromptDraft::from_text(text));
    keys(&mut state, "ESC g g 0");
    state
}

fn set_draft(state: &mut AppState, draft: PromptDraft) {
    let now = state.clock;
    state
        .composers
        .get_mut(THREAD)
        .expect("composer")
        .set_draft(draft, now);
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

fn register(state: &AppState, name: char) -> Option<String> {
    state
        .registers
        .get(Some(name))
        .map(|value| value.text.clone())
}

fn echo(state: &AppState) -> Option<String> {
    state.keys.echo.as_ref().map(|echo| echo.text.clone())
}

fn path(name: &str) -> PromptMentionResource {
    PromptMentionResource::Path {
        source: PromptMentionPathSource::Workspace,
        entry_kind: PathEntryKind::File,
        path: name.into(),
        label: name.into(),
    }
}

fn chip_draft(text: &str, chip: &str) -> PromptDraft {
    let start = text.find(chip).expect("chip in text");
    let start_chars = text[..start].chars().count();
    let end_chars = start_chars + chip.chars().count();
    PromptDraft {
        text: text.into(),
        mentions: vec![PromptTextMention {
            start: char_to_utf16(text, start_chars) as u32,
            end: char_to_utf16(text, end_chars) as u32,
            resource: path(chip),
        }],
        attachments: Vec::new(),
    }
}

fn mentions(state: &AppState) -> Vec<(u32, u32)> {
    state
        .composer()
        .expect("composer")
        .draft()
        .mentions
        .iter()
        .map(|mention| (mention.start, mention.end))
        .collect()
}

fn ctrl(state: &mut AppState, ch: char) -> Vec<Effect> {
    update(
        state,
        Action::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)),
    )
}

#[test]
fn dot_repeats_operators_with_motions_and_new_counts() {
    let mut state = with_text("one two three four five six");
    keys(&mut state, "d w");
    assert_eq!(text(&state), "two three four five six");
    keys(&mut state, ".");
    assert_eq!(text(&state), "three four five six");
    keys(&mut state, "2 .");
    assert_eq!(text(&state), "five six");
    keys(&mut state, "u");
    assert_eq!(text(&state), "three four five six");
    keys(&mut state, "u u");
    assert_eq!(text(&state), "one two three four five six");
}

#[test]
fn dot_replays_change_and_insert_text() {
    let mut state = with_text("foo bar baz");
    keys(&mut state, "c i w");
    type_chars(&mut state, "qux");
    keys(&mut state, "ESC w .");
    assert_eq!(text(&state), "qux qux baz");
    keys(&mut state, "w .");
    assert_eq!(text(&state), "qux qux qux");
    keys(&mut state, "u");
    assert_eq!(text(&state), "qux qux baz");

    let mut state = with_text("x");
    keys(&mut state, "A");
    type_chars(&mut state, "ab");
    keys(&mut state, "ESC 3 .");
    assert_eq!(text(&state), "xabababab");
    keys(&mut state, "o");
    type_chars(&mut state, "line");
    keys(&mut state, "ESC .");
    assert_eq!(text(&state), "xabababab\nline\nline");
    assert_eq!(mode(&state), EditMode::Normal);
}

#[test]
fn dot_repeats_char_commands_and_visual_operations() {
    let mut state = with_text("abcdefgh");
    keys(&mut state, "x .");
    assert_eq!(text(&state), "cdefgh");
    keys(&mut state, "r z l .");
    assert_eq!(text(&state), "zzefgh");
    keys(&mut state, "0 v l d");
    assert_eq!(text(&state), "efgh");
    keys(&mut state, ".");
    assert_eq!(text(&state), "gh");
    let mut state = with_text("a\nb\nc\nd\ne");
    keys(&mut state, "V j d");
    assert_eq!(text(&state), "c\nd\ne");
    keys(&mut state, ".");
    assert_eq!(text(&state), "e");
    let mut state = with_text("aa bb cc");
    keys(&mut state, "~ ~ w g U i w w .");
    assert_eq!(text(&state), "AA BB CC");
    keys(&mut state, "d t C");
    assert_eq!(text(&state), "AA BB CC");
    keys(&mut state, "0 d f B .");
    assert_eq!(text(&state), "B CC");
}

#[test]
fn replace_state_overwrites_and_backspace_restores() {
    let mut state = with_text("hello world");
    keys(&mut state, "w R");
    assert_eq!(mode(&state), EditMode::Replace);
    assert_eq!(tag(&state), Tag::Replace);
    let lines = render_lines(&state, 140, 30);
    let status = &lines[lines.len() - 2];
    assert!(status.trim_end().ends_with("<R>"), "{status}");
    type_chars(&mut state, "WORLD!!");
    assert_eq!(text(&state), "hello WORLD!!");
    press(&mut state, KeyCode::Backspace);
    press(&mut state, KeyCode::Backspace);
    press(&mut state, KeyCode::Backspace);
    assert_eq!(text(&state), "hello WORLd");
    keys(&mut state, "ESC");
    assert_eq!(mode(&state), EditMode::Normal);
    assert_eq!(cursor(&state), 9);
    keys(&mut state, "u");
    assert_eq!(text(&state), "hello world");
    keys(&mut state, "0 R");
    type_chars(&mut state, "J");
    keys(&mut state, "ESC w .");
    assert_eq!(text(&state), "Jello Jorld");
}

#[test]
fn undo_line_restores_the_last_changed_line_and_toggles() {
    let mut state = with_text("first line\nsecond line");
    keys(&mut state, "j x x w d w");
    assert_eq!(text(&state), "first line\ncond ");
    keys(&mut state, "U");
    assert_eq!(text(&state), "first line\nsecond line");
    keys(&mut state, "U");
    assert_eq!(text(&state), "first line\ncond ");
    keys(&mut state, "u");
    assert_eq!(text(&state), "first line\nsecond line");
}

#[test]
fn gv_reselects_the_last_visual_selection() {
    let mut state = with_text("alpha beta gamma");
    keys(&mut state, "w v e ESC 0");
    assert_eq!(mode(&state), EditMode::Normal);
    keys(&mut state, "g v");
    assert!(matches!(
        mode(&state),
        EditMode::Visual {
            line: false,
            anchor: 6,
            head: 9
        }
    ));
    let effects = keys(&mut state, "y");
    assert_eq!(effects, vec![Effect::Copy("beta".into())]);
    let mut fresh = with_text("abc");
    keys(&mut fresh, "g v");
    assert_eq!(mode(&fresh), EditMode::Normal);
    assert_eq!(
        echo(&fresh).as_deref(),
        Some("No previous visual selection")
    );
}

#[test]
fn marks_jump_and_bound_operators() {
    let mut state = with_text("one\n  two\nthree\nfour");
    keys(&mut state, "j l l m a G");
    assert_eq!(cursor(&state), 16);
    keys(&mut state, "' a");
    assert_eq!(cursor(&state), 6);
    keys(&mut state, "G ` a");
    assert_eq!(cursor(&state), 6);
    keys(&mut state, "' '");
    assert_eq!(cursor(&state), 16);
    keys(&mut state, "' '");
    assert_eq!(cursor(&state), 6);
    keys(&mut state, "G d ' a");
    assert_eq!(text(&state), "one");
    keys(&mut state, "u");
    assert_eq!(text(&state), "one\n  two\nthree\nfour");
    keys(&mut state, "g g ` a");
    keys(&mut state, "0 y ` a");
    assert_eq!(register(&state, '0').as_deref(), Some("  "));
    keys(&mut state, "' z");
    assert_eq!(echo(&state).as_deref(), Some("E20: Mark not set"));
    keys(&mut state, "m 1");
    assert_eq!(
        echo(&state).as_deref(),
        Some("E191: Argument must be a letter or forward/backward quote")
    );
}

#[test]
fn marks_follow_edits_and_special_marks_track_changes() {
    let mut state = with_text("aaa bbb ccc");
    keys(&mut state, "$ m c 0 d w");
    assert_eq!(text(&state), "bbb ccc");
    keys(&mut state, "` c");
    assert_eq!(cursor(&state), 6);
    keys(&mut state, "0 w c w");
    type_chars(&mut state, "XYZW");
    keys(&mut state, "ESC 0 ` [");
    assert_eq!(cursor(&state), 4);
    keys(&mut state, "0 ` ^");
    assert_eq!(cursor(&state), 7);
    keys(&mut state, "0 v e ESC $ ` <");
    assert_eq!(cursor(&state), 0);
    keys(&mut state, "` >");
    assert_eq!(cursor(&state), 2);
}

#[test]
fn numbered_named_and_small_delete_registers() {
    let mut state = with_text("l1\nl2\nl3\nl4 word");
    keys(&mut state, "d d d d");
    assert_eq!(register(&state, '1').as_deref(), Some("l2\n"));
    assert_eq!(register(&state, '2').as_deref(), Some("l1\n"));
    keys(&mut state, "j w d w");
    assert_eq!(register(&state, '-').as_deref(), Some("word"));
    assert_eq!(register(&state, '1').as_deref(), Some("l2\n"));
    keys(&mut state, "\" 2 p");
    assert_eq!(text(&state), "l3\nl4 \nl1");
    keys(&mut state, "g g \" a y w \" A y y");
    assert_eq!(register(&state, 'a').as_deref(), Some("l3l3\n"));
    keys(&mut state, "\" a P");
    assert_eq!(text(&state), "l3l3\nl3\nl4 \nl1");
}

#[test]
fn read_only_registers_put_but_refuse_writes() {
    let mut state = with_text("x");
    keys(&mut state, "A");
    type_chars(&mut state, "yz");
    keys(&mut state, "ESC \" . p");
    assert_eq!(text(&state), "xyzyz");
    assert_eq!(register(&state, '.').as_deref(), Some("yz"));
    keys(&mut state, "\" . y w");
    assert_eq!(
        echo(&state).as_deref(),
        Some("E354: Invalid register name: '.'")
    );
    keys(&mut state, "\" . d d");
    assert_eq!(text(&state), "xyzyz");
    keys(&mut state, ": n o h RET");
    assert_eq!(register(&state, ':').as_deref(), Some("noh"));
    keys(&mut state, "0 \" : P");
    assert_eq!(text(&state), "nohxyzyz");
    keys(&mut state, "A C-r .");
    assert_eq!(text(&state), "nohxyzyzyz");
}

#[test]
fn clipboard_register_gets_yanks_and_bracketed_pastes() {
    let mut state = with_text("copy me");
    let effects = keys(&mut state, "y i w");
    assert_eq!(effects, vec![Effect::Copy("copy".into())]);
    assert_eq!(register(&state, '+').as_deref(), Some("copy"));
    keys(&mut state, "d w");
    assert_eq!(register(&state, '+').as_deref(), Some("copy"));
    update(&mut state, Action::Paste("from outside".into()));
    assert_eq!(register(&state, '+').as_deref(), Some("from outside"));
    assert_eq!(mode(&state), EditMode::Normal);
    assert_eq!(text(&state), "from outsideme");
    assert_eq!(cursor(&state), 11);
    keys(&mut state, "u");
    assert_eq!(text(&state), "me");
    keys(&mut state, "\" + p");
    assert_eq!(text(&state), "mfrom outsidee");
    let effects = keys(&mut state, "\" * y y");
    assert_eq!(effects, vec![Effect::Copy("mfrom outsidee\n".into())]);
}

#[test]
fn bracketed_paste_replaces_a_visual_selection_and_joins_insert_sessions() {
    let mut state = with_text("keep drop keep");
    keys(&mut state, "w v e");
    update(&mut state, Action::Paste("new".into()));
    assert_eq!(text(&state), "keep new keep");
    assert_eq!(mode(&state), EditMode::Normal);
    keys(&mut state, "u");
    assert_eq!(text(&state), "keep drop keep");
    keys(&mut state, "A");
    type_chars(&mut state, " a");
    update(&mut state, Action::Paste("\r\nb".into()));
    type_chars(&mut state, "c");
    keys(&mut state, "ESC");
    assert_eq!(text(&state), "keep drop keep a\nbc");
    keys(&mut state, "u");
    assert_eq!(text(&state), "keep drop keep");
}

#[test]
fn unicode_operators_respect_grapheme_clusters() {
    let mut state = with_text("e\u{301}tude 👨\u{200d}👩\u{200d}👧 ok");
    keys(&mut state, "x");
    assert_eq!(text(&state), "tude 👨\u{200d}👩\u{200d}👧 ok");
    keys(&mut state, "w x");
    assert_eq!(text(&state), "tude  ok");
    keys(&mut state, "u r Z");
    assert_eq!(text(&state), "tude Z ok");
    keys(&mut state, "u 0 ~ ~");
    assert_eq!(text(&state), "TUde 👨\u{200d}👩\u{200d}👧 ok");
    keys(&mut state, "$ X");
    assert_eq!(text(&state), "TUde 👨\u{200d}👩\u{200d}👧 k");
    keys(&mut state, "a");
    type_chars(&mut state, "!");
    keys(&mut state, "ESC");
    assert_eq!(text(&state), "TUde 👨\u{200d}👩\u{200d}👧 k!");
}

#[test]
fn cyrillic_and_cjk_words_take_operators_and_counts() {
    let mut state = with_text("Привет, мир! 你好世界 ещё");
    keys(&mut state, "d w");
    assert_eq!(text(&state), ", мир! 你好世界 ещё");
    keys(&mut state, "w c w");
    type_chars(&mut state, "друг");
    keys(&mut state, "ESC");
    assert_eq!(text(&state), ", друг! 你好世界 ещё");
    keys(&mut state, "2 w g U i w");
    assert_eq!(text(&state), ", друг! 你好世界 ещё");
    keys(&mut state, "w g U i w");
    assert_eq!(text(&state), ", друг! 你好世界 ЕЩЁ");
    keys(&mut state, "b d i w");
    assert_eq!(text(&state), ", друг!  ЕЩЁ");
    keys(&mut state, "0 f д y e");
    assert_eq!(register(&state, '0').as_deref(), Some("друг"));
    keys(&mut state, "$ F д D");
    assert_eq!(text(&state), ", ");
}

#[test]
fn sentence_text_objects_and_motions_in_the_composer() {
    let mut state = with_text("First one. Second one. Third.");
    keys(&mut state, ")");
    assert_eq!(cursor(&state), 11);
    keys(&mut state, "d a s");
    assert_eq!(text(&state), "First one. Third.");
    keys(&mut state, "(");
    assert_eq!(cursor(&state), 0);
    keys(&mut state, "c i s");
    type_chars(&mut state, "New.");
    keys(&mut state, "ESC");
    assert_eq!(text(&state), "New. Third.");
}

#[test]
fn mention_chips_are_atomic_for_motions_and_deletes() {
    let text_with_chip = "ask 😀 @src/main.rs now";
    let mut state = with_text("");
    set_draft(&mut state, chip_draft(text_with_chip, "@src/main.rs"));
    keys(&mut state, "ESC 0");
    assert_eq!(mentions(&state), vec![(7, 19)]);
    keys(&mut state, "w w");
    assert_eq!(cursor(&state), 6);
    keys(&mut state, "w");
    assert_eq!(cursor(&state), 19);
    keys(&mut state, "b");
    assert_eq!(cursor(&state), 6);
    keys(&mut state, "l");
    assert_eq!(cursor(&state), 18);
    keys(&mut state, "h");
    assert_eq!(cursor(&state), 6);
    keys(&mut state, "x");
    assert_eq!(text(&state), "ask 😀  now");
    assert!(mentions(&state).is_empty());
    keys(&mut state, "u");
    assert_eq!(text(&state), text_with_chip);
    assert_eq!(mentions(&state), vec![(7, 19)]);
    keys(&mut state, "0 d w");
    assert_eq!(text(&state), "😀 @src/main.rs now");
    assert_eq!(mentions(&state), vec![(3, 15)]);
    keys(&mut state, "0 l v l d");
    assert_eq!(text(&state), "😀 now");
    assert!(mentions(&state).is_empty());
    keys(&mut state, "u 0 2 l y e");
    assert_eq!(register(&state, '0').as_deref(), Some("@src/main.rs"));
}

#[test]
fn mention_offsets_stay_utf16_after_edits_before_the_chip() {
    let text_with_chip = "👨\u{200d}👩\u{200d}👧 x @a.rs end";
    let mut state = with_text("");
    set_draft(&mut state, chip_draft(text_with_chip, "@a.rs"));
    keys(&mut state, "ESC 0");
    assert_eq!(mentions(&state), vec![(11, 16)]);
    keys(&mut state, "w x");
    assert_eq!(mentions(&state), vec![(10, 15)]);
    keys(&mut state, "0 i");
    type_chars(&mut state, "😀");
    keys(&mut state, "ESC");
    assert_eq!(mentions(&state), vec![(12, 17)]);
    keys(&mut state, "$ d b");
    assert_eq!(mentions(&state), vec![(12, 17)]);
    assert!(text(&state).ends_with("@a.rs d"));
}

#[test]
fn insert_keys_literal_tab_and_first_non_blank() {
    let mut state = composing();
    type_chars(&mut state, "  indented");
    ctrl(&mut state, 'a');
    type_chars(&mut state, "<");
    assert_eq!(text(&state), "  <indented");
    ctrl(&mut state, 'v');
    press(&mut state, KeyCode::Tab);
    assert_eq!(text(&state), "  <\tindented");
    ctrl(&mut state, 'v');
    press(&mut state, KeyCode::Enter);
    assert_eq!(text(&state), "  <\t\nindented");
    press(&mut state, KeyCode::Tab);
    assert_eq!(text(&state), "  <\t\n\tindented");
    ctrl(&mut state, 'e');
    ctrl(&mut state, 'v');
    type_chars(&mut state, "j");
    assert_eq!(text(&state), "  <\t\n\tindentedj");
}

#[test]
fn counts_registers_and_operators_combine() {
    let mut state = with_text("a b c d e f");
    keys(&mut state, "\" q 3 d w");
    assert_eq!(text(&state), "d e f");
    assert_eq!(register(&state, 'q').as_deref(), Some("a b c "));
    keys(&mut state, "$ \" q p");
    assert_eq!(text(&state), "d e fa b c ");
    keys(&mut state, "0 2 y 2 w");
    assert_eq!(register(&state, '0').as_deref(), Some("d e fa b "));
    keys(&mut state, "5 x");
    assert_eq!(text(&state), "a b c ");
    keys(&mut state, "g g 3 J");
    assert_eq!(text(&state), "a b c ");
}

#[test]
fn clicking_the_editor_puts_the_cursor_there_and_dragging_selects() {
    let mut state = composing();
    type_chars(&mut state, "hello world");
    keys(&mut state, "ESC");
    let width = 100;
    let height = 24;
    render_lines(&state, width, height);
    let editor = state
        .mouse_areas
        .editors
        .borrow()
        .iter()
        .find(|(thread, _)| thread == THREAD)
        .map(|(_, area)| *area)
        .expect("editor area");
    let mouse = |state: &mut AppState, kind, column, row| {
        update(
            state,
            Action::Mouse(MouseEvent {
                kind,
                column,
                row,
                modifiers: KeyModifiers::NONE,
            }),
        )
    };
    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        editor.x + 6,
        editor.y,
    );
    assert_eq!(mode(&state), EditMode::Insert);
    assert_eq!(cursor(&state), 6);
    mouse(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        editor.x + 10,
        editor.y,
    );
    assert!(matches!(
        mode(&state),
        EditMode::Visual {
            line: false,
            anchor: 6,
            head: 10
        }
    ));
    let effects = keys(&mut state, "y");
    assert_eq!(effects, vec![Effect::Copy("world".into())]);
}
