use crossterm::event::{KeyCode, KeyModifiers};

use crate::keymap::{Act, Command, Direction, JumpKind, KeyChord, Keymap, Scope, ScreenPos};
use crate::rbuf::{Batch, ViewOp};
use crate::vi::{CharAct, EditOp, InsertAt, Motion, Object, Operator, Target};

fn cmd(command: Command) -> Act {
    Act::Cmd(command)
}

fn key(token: &str) -> Act {
    Act::Key(KeyChord::parse(token).unwrap_or_else(|| panic!("bad key {token}")))
}

fn edit(op: EditOp) -> Act {
    Act::Cmd(Command::Edit(op))
}

fn view(op: ViewOp) -> Act {
    Act::Cmd(Command::View(op))
}

pub(crate) fn install(map: &mut Keymap) {
    direct_chords(map);
    global(map);
    leader(map);
    windows(map, "C-w");
    windows(map, "SPC w");
    sidebar(map);
    timeline(map);
    composer(map);
    visual(map);
    insert(map);
    new_thread(map);
    panel(map);
    views(map);
    read_only(map, Scope::Sidebar, false);
    sidebar_overrides(map);
    read_only(map, Scope::Timeline, true);
    timeline_overrides(map);
    for scope in [
        Scope::PanelDiff,
        Scope::PanelFile,
        Scope::PanelTerminal,
        Scope::Pane,
    ] {
        read_only(map, scope, true);
    }
    read_only_overrides(map);
    view_visual(map);
}

fn vertical() -> Vec<(&'static str, ViewOp)> {
    use crate::rbuf::ViewOp::*;
    vec![
        ("j", Move(Motion::Down)),
        ("↓", Move(Motion::Down)),
        ("k", Move(Motion::Up)),
        ("↑", Move(Motion::Up)),
        ("g j", Move(Motion::Down)),
        ("g k", Move(Motion::Up)),
        ("g g", Move(Motion::FirstLine)),
        ("G", Move(Motion::LastLine)),
        ("Home", Move(Motion::FirstLine)),
        ("End", Move(Motion::LastLine)),
        (
            "C-d",
            Page {
                down: true,
                half: true,
            },
        ),
        (
            "C-u",
            Page {
                down: false,
                half: true,
            },
        ),
        (
            "C-f",
            Page {
                down: true,
                half: false,
            },
        ),
        (
            "C-b",
            Page {
                down: false,
                half: false,
            },
        ),
        (
            "PgDn",
            Page {
                down: true,
                half: false,
            },
        ),
        (
            "PgUp",
            Page {
                down: false,
                half: false,
            },
        ),
        ("C-e", Scroll(1)),
        ("C-y", Scroll(-1)),
        ("z z", Align(ScreenPos::Middle)),
        ("z .", Align(ScreenPos::Middle)),
        ("z t", Align(ScreenPos::Top)),
        ("z RET", Align(ScreenPos::Top)),
        ("z b", Align(ScreenPos::Bottom)),
        ("z -", Align(ScreenPos::Bottom)),
        ("H", Screen(ScreenPos::Top)),
        ("M", Screen(ScreenPos::Middle)),
        ("L", Screen(ScreenPos::Bottom)),
        ("/", Search { forward: true }),
        ("?", Search { forward: false }),
        ("n", SearchNext { reverse: false }),
        ("N", SearchNext { reverse: true }),
        (
            "*",
            SearchWord {
                forward: true,
                whole: true,
            },
        ),
        (
            "#",
            SearchWord {
                forward: false,
                whole: true,
            },
        ),
        (
            "g *",
            SearchWord {
                forward: true,
                whole: false,
            },
        ),
        (
            "g #",
            SearchWord {
                forward: false,
                whole: false,
            },
        ),
    ]
}

fn read_only(map: &mut Keymap, scope: Scope, columns: bool) {
    if columns {
        for (keys, motion) in motions() {
            map.bind(scope, keys, view(ViewOp::Move(motion)));
            map.bind(
                scope,
                &format!("y {keys}"),
                view(ViewOp::Yank(Target::Motion(motion))),
            );
        }
        for (keys, forward, till) in finds() {
            map.bind(scope, keys, Act::Char(CharAct::Find { forward, till }));
            map.bind(
                scope,
                &format!("y {keys}"),
                Act::Char(CharAct::ApplyFind {
                    op: Operator::Yank,
                    forward,
                    till,
                }),
            );
        }
        for (keys, object) in objects() {
            map.bind(
                scope,
                &format!("y i {keys}"),
                view(ViewOp::Yank(Target::Inner(object))),
            );
            map.bind(
                scope,
                &format!("y a {keys}"),
                view(ViewOp::Yank(Target::Around(object))),
            );
        }
        map.group(scope, "y i", "+inner");
        map.group(scope, "y a", "+around");
        map.bind(
            scope,
            "Y",
            view(ViewOp::Yank(Target::Motion(Motion::LineEnd))),
        );
    } else {
        map.bind(scope, "Y", view(ViewOp::Yank(Target::Line)));
    }
    for (keys, op) in vertical() {
        map.bind(scope, keys, view(op));
    }
    for keys in ["j", "k", "g g", "G"] {
        let motion = match keys {
            "j" => Motion::Down,
            "k" => Motion::Up,
            "g g" => Motion::FirstLine,
            _ => Motion::LastLine,
        };
        map.bind(
            scope,
            &format!("y {keys}"),
            view(ViewOp::Yank(Target::Motion(motion))),
        );
    }
    map.bind(scope, "y y", view(ViewOp::Yank(Target::Line)));
    map.bind(scope, "v", view(ViewOp::Visual { line: !columns }));
    map.bind(scope, "V", view(ViewOp::Visual { line: true }));
    map.bind(scope, "g v", view(ViewOp::Reselect));
    map.bind(scope, "C-o", view(ViewOp::JumpOlder));
    map.bind(scope, "C-i", view(ViewOp::JumpNewer));
    map.bind(scope, "\"", Act::Char(CharAct::Register));
    map.bind(scope, "m", Act::Char(CharAct::SetMark));
    map.bind(scope, "'", Act::Char(CharAct::Mark { line: true }));
    map.bind(scope, "`", Act::Char(CharAct::Mark { line: false }));
    map.bind(
        scope,
        "y '",
        Act::Char(CharAct::ApplyMark {
            op: Operator::Yank,
            line: true,
        }),
    );
    map.bind(
        scope,
        "y `",
        Act::Char(CharAct::ApplyMark {
            op: Operator::Yank,
            line: false,
        }),
    );
    map.group(scope, "y", "+yank");
}

fn sidebar_overrides(map: &mut Keymap) {
    use Command::*;
    let s = Scope::Sidebar;
    let entries: &[(&str, Command)] = &[
        ("TAB", ToggleGroup),
        ("z a", ToggleGroup),
        ("z o", Expand),
        ("z O", Expand),
        ("z c", Collapse),
        ("z C", Collapse),
        ("z R", FoldAll(true)),
        ("z M", FoldAll(false)),
        ("y y", YankTitle),
        ("g y", YankId),
    ];
    for (keys, command) in entries {
        map.bind(s, keys, cmd(*command));
    }
}

fn timeline_overrides(map: &mut Keymap) {
    use Command::*;
    let t = Scope::Timeline;
    let entries: &[(&str, Command)] = &[
        ("{", MoveUp),
        ("}", MoveDown),
        ("RET", Open),
        ("TAB", ToggleGroup),
        ("z a", ToggleGroup),
        ("z o", Expand),
        ("z O", Expand),
        ("z c", Collapse),
        ("z C", Collapse),
        ("z R", FoldAll(true)),
        ("z M", FoldAll(false)),
        ("g r", Reload),
        ("] m", NextMessage),
        ("[ m", PreviousMessage),
        ("] ]", TimelineJump(JumpKind::Turn, true)),
        ("[ [", TimelineJump(JumpKind::Turn, false)),
        ("] e", TimelineJump(JumpKind::Error, true)),
        ("[ e", TimelineJump(JumpKind::Error, false)),
        ("g y", Copy),
        ("g x", OpenExternal),
        ("g f", PreviewAtCursor),
        ("i", ComposerInsert(InsertAt::After)),
        ("a", ComposerInsert(InsertAt::After)),
        ("A", ComposerInsert(InsertAt::LineEnd)),
        ("o", ComposerInsert(InsertAt::Below)),
        ("O", ComposerInsert(InsertAt::Above)),
        ("I", ComposerInsert(InsertAt::LineStart)),
        ("g i", ComposerInsert(InsertAt::Last)),
    ];
    for (keys, command) in entries {
        map.bind(t, keys, cmd(*command));
    }
    map.bind(t, "y i m", view(ViewOp::YankRow { around: false }));
    map.bind(t, "y a m", view(ViewOp::YankRow { around: true }));
}

fn read_only_overrides(map: &mut Keymap) {
    passthrough(
        map,
        Scope::PanelDiff,
        &[
            ("RET", "RET", "Fold / unfold the file, load the diff"),
            ("TAB", "RET", "Fold / unfold the file"),
            ("z a", "RET", "Fold / unfold the file"),
            ("] c", "n", "Next hunk"),
            ("[ c", "N", "Previous hunk"),
            ("] f", "]", "Next file"),
            ("[ f", "[", "Previous file"),
            ("s", "s", "Next scope"),
            ("S", "S", "Previous scope"),
            ("g f", "p", "Preview the file at the cursor line"),
            ("e", "e", "Open the file in $EDITOR"),
        ],
    );
    map.bind_desc(
        Scope::PanelDiff,
        "g y",
        cmd(Command::FileYankPath),
        "Yank the file path",
    );
    map.bind(Scope::PanelDiff, "z R", cmd(Command::FoldAll(true)));
    map.bind(Scope::PanelDiff, "z M", cmd(Command::FoldAll(false)));
    for scope in [Scope::PanelDiff, Scope::PanelFile, Scope::Pane] {
        map.bind_desc(scope, "z h", key("h"), "Scroll left");
        map.bind_desc(scope, "z l", key("l"), "Scroll right");
    }
    passthrough(
        map,
        Scope::PanelFile,
        &[
            ("e", "e", "Open the file in $EDITOR"),
            ("g y", "y", "Yank the path"),
        ],
    );
    passthrough(
        map,
        Scope::PanelTerminal,
        &[
            ("RET", "i", "Type into the terminal"),
            ("i", "i", "Type into the terminal"),
            ("a", "i", "Type into the terminal"),
            ("I", "i", "Type into the terminal"),
            ("A", "i", "Type into the terminal"),
            ("] t", "]", "Next terminal"),
            ("[ t", "[", "Previous terminal"),
            ("o", "n", "Start a new terminal"),
            ("d d", "x", "Close the terminal"),
            ("x", "x", "Close the terminal"),
        ],
    );
    passthrough(
        map,
        Scope::Pane,
        &[
            ("g y", "y", "Yank the output or path"),
            ("g x", "o", "Open the previewed file"),
            ("q", "ESC", "Close"),
            ("RET", "ESC", "Close"),
        ],
    );
}

fn view_visual(map: &mut Keymap) {
    let v = Scope::ViewVisual;
    for (keys, motion) in motions() {
        map.bind(v, keys, view(ViewOp::Move(motion)));
    }
    for (keys, forward, till) in finds() {
        map.bind(v, keys, Act::Char(CharAct::Find { forward, till }));
    }
    for (keys, op) in vertical() {
        map.bind(v, keys, view(op));
    }
    for (keys, object) in objects() {
        map.bind(
            v,
            &format!("i {keys}"),
            view(ViewOp::VisualObject(Target::Inner(object))),
        );
        map.bind(
            v,
            &format!("a {keys}"),
            view(ViewOp::VisualObject(Target::Around(object))),
        );
    }
    map.bind(v, "i m", view(ViewOp::VisualRow { around: false }));
    map.bind(v, "a m", view(ViewOp::VisualRow { around: true }));
    map.group(v, "i", "+inner");
    map.group(v, "a", "+around");
    let entries: &[(&str, ViewOp)] = &[
        ("y", ViewOp::VisualYank { line: false }),
        ("Y", ViewOp::VisualYank { line: true }),
        ("o", ViewOp::VisualSwap),
        ("O", ViewOp::VisualSwap),
        ("v", ViewOp::Visual { line: false }),
        ("V", ViewOp::Visual { line: true }),
        ("d", ViewOp::Batch(Batch::Archive)),
        ("x", ViewOp::Batch(Batch::Archive)),
        ("Del", ViewOp::Batch(Batch::Archive)),
        ("!", ViewOp::Batch(Batch::Read)),
        ("+", ViewOp::Batch(Batch::Pin)),
    ];
    for (keys, op) in entries {
        map.bind(v, keys, view(*op));
    }
    map.bind(v, "\"", Act::Char(CharAct::Register));
    map.bind(v, "m", Act::Char(CharAct::SetMark));
    map.bind(v, "'", Act::Char(CharAct::Mark { line: true }));
    map.bind(v, "`", Act::Char(CharAct::Mark { line: false }));
}

fn direct_chords(map: &mut Keymap) {
    use Command::*;
    let entries = [
        (KeyChord::plain(KeyCode::Esc), Back),
        (KeyChord::alt('x'), PaletteOpen),
        (KeyChord::alt('h'), PaneLeft),
        (KeyChord::new(KeyCode::Left, KeyModifiers::ALT), PaneLeft),
        (KeyChord::alt('l'), PaneRight),
        (KeyChord::new(KeyCode::Right, KeyModifiers::ALT), PaneRight),
        (KeyChord::alt('z'), PaneMaximize),
        (KeyChord::alt('m'), ModelPicker),
        (KeyChord::alt('e'), ReasoningPicker),
        (KeyChord::alt('p'), PermissionPicker),
        (KeyChord::alt('.'), CycleModel),
        (KeyChord::alt(','), CycleModelBackward),
        (KeyChord::alt('r'), CycleReasoning),
        (
            KeyChord::new(KeyCode::Char('r'), KeyModifiers::ALT | KeyModifiers::SHIFT),
            CycleReasoningBackward,
        ),
        (KeyChord::alt('q'), QueueList),
        (KeyChord::alt('t'), ToggleSubmitMode),
        (KeyChord::alt('w'), NextPending),
        (KeyChord::alt('a'), Attachments),
        (KeyChord::plain(KeyCode::F(1)), Help),
    ];
    for (chord, command) in entries {
        map.direct(chord, command);
    }
    for n in 1..=9u8 {
        map.direct(KeyChord::alt(char::from(b'0' + n)), Jump(n));
    }
}

fn global(map: &mut Keymap) {
    use Command::*;
    let g = Scope::Global;
    map.bind(g, ":", cmd(ExLine));
    map.bind(g, "@ :", cmd(ExRepeat));
    map.bind(g, "C-g", cmd(Back));
    map.bind(g, "q", cmd(QuitHint));
    map.bind(g, "C-h", cmd(WindowFocus(Direction::Left)));
    map.bind(g, "C-j", cmd(WindowFocus(Direction::Down)));
    map.bind(g, "C-k", cmd(WindowFocus(Direction::Up)));
    map.bind(g, "C-l", cmd(WindowFocus(Direction::Right)));
    map.bind(g, "g t", cmd(PanelNextTab));
    map.bind(g, "g T", cmd(PanelPreviousTab));
    map.bind(g, "] b", cmd(ThreadNext));
    map.bind(g, "[ b", cmd(ThreadPrevious));
    map.bind(g, "] w", cmd(StepWaiting(true)));
    map.bind(g, "[ w", cmd(StepWaiting(false)));
    map.bind(g, "Z Z", cmd(SendAndClose));
    map.bind(g, "Z Q", cmd(CloseWindowOrQuit));
    map.bind(g, "C-c C-c", cmd(Send));
    map.bind(g, "C-c C-s", cmd(Steer));
    map.bind(g, "C-c C-k", cmd(StopTurn));
    map.bind(g, "C-c C-a", cmd(AttachFile));
    map.group(g, "g", "+goto");
    map.group(g, "z", "+scroll/fold");
    map.group(g, "]", "+next");
    map.group(g, "[", "+previous");
    map.group(g, "Z", "+quit");
    map.group(g, "C-c", "+thread");
    map.group(g, "C-w", "+window");
    map.group(g, "SPC", "+leader");
}

fn leader(map: &mut Keymap) {
    use Command::*;
    let g = Scope::Global;
    let groups = [
        ("SPC a", "+agent"),
        ("SPC b", "+buffer"),
        ("SPC f", "+file"),
        ("SPC g", "+git"),
        ("SPC g c", "+commit"),
        ("SPC g p", "+pull request"),
        ("SPC h", "+help"),
        ("SPC h b", "+bindings"),
        ("SPC m", "+thread"),
        ("SPC n", "+notifications"),
        ("SPC o", "+open"),
        ("SPC p", "+project"),
        ("SPC q", "+quit"),
        ("SPC s", "+search"),
        ("SPC t", "+toggle"),
        ("SPC w", "+window"),
        ("SPC w m", "+maximize"),
        ("SPC TAB", "+workspace"),
    ];
    for (keys, name) in groups {
        map.group(g, keys, name);
    }
    let entries: &[(&str, Command)] = &[
        ("SPC SPC", QuickOpen),
        ("SPC .", QuickOpen),
        ("SPC ,", SearchThreads),
        ("SPC <", SearchThreads),
        ("SPC /", SearchThreads),
        ("SPC :", PaletteOpen),
        ("SPC ;", ExLine),
        ("SPC '", ResumeMinibuffer),
        ("SPC `", AlternateThread),
        ("SPC x", NewThread),
        ("SPC a a", NewThread),
        ("SPC a t", ComposerToggle),
        ("SPC a c", FocusComposer),
        ("SPC a r", SearchThreads),
        ("SPC a p", Send),
        ("SPC a m", ComposerMention),
        ("SPC a l", SearchThreads),
        ("SPC a k", StatusPanel),
        ("SPC a q", StopTurn),
        ("SPC a .", PaletteThreadGroup),
        ("SPC b b", SearchThreads),
        ("SPC b B", SearchThreads),
        ("SPC b n", ThreadNext),
        ("SPC b ]", ThreadNext),
        ("SPC b p", ThreadPrevious),
        ("SPC b [", ThreadPrevious),
        ("SPC b l", AlternateThread),
        ("SPC b N", NewThread),
        ("SPC b c", NewThreadInEnvironment),
        ("SPC b k", Archive),
        ("SPC b d", Archive),
        ("SPC b u", UndoArchive),
        ("SPC b R", Rename),
        ("SPC b r", Reload),
        ("SPC b m", TogglePin),
        ("SPC b !", ToggleRead),
        ("SPC b y", YankTimeline),
        ("SPC b i", SearchThreads),
        ("SPC f f", QuickOpen),
        ("SPC f y", FileYankPath),
        ("SPC f e", FileEdit),
        ("SPC f a", AttachFile),
        ("SPC f A", Attachments),
        ("SPC g g", DiffToggle),
        ("SPC g c c", GitCommit),
        ("SPC g p m", PullRequestMerge),
        ("SPC g p r", PullRequestReady),
        ("SPC g p d", PullRequestDraft),
        ("SPC g ]", DiffHunk(true)),
        ("SPC g [", DiffHunk(false)),
        ("SPC h b b", Help),
        ("SPC h k", DescribeKey),
        ("SPC h e", Messages),
        ("SPC h ,", Settings),
        ("SPC h ?", KeymapOverview),
        ("SPC m s", Send),
        ("SPC m S", Steer),
        ("SPC m k", StopTurn),
        ("SPC m q", QueueList),
        ("SPC m r", NextPending),
        ("SPC m m", ModelPicker),
        ("SPC m e", ReasoningPicker),
        ("SPC m p", PermissionPicker),
        ("SPC m a", AttachFile),
        ("SPC m A", Attachments),
        ("SPC m n", NewThreadInEnvironment),
        ("SPC m i", StatusPanel),
        ("SPC n n", Notifications),
        ("SPC n r", NextPending),
        ("SPC n c", ClearNotifications),
        ("SPC o p", SidebarToggle),
        ("SPC o P", RevealInSidebar),
        ("SPC o i", PanelToggle),
        ("SPC o d", DiffToggle),
        ("SPC o t", TerminalOpen),
        ("SPC o T", TerminalNew),
        ("SPC o f", TerminalAttach),
        ("SPC o A", Automations),
        ("SPC o N", NewAutomation),
        ("SPC o x", OpenExternal),
        ("SPC p p", SwitchProject),
        ("SPC p f", QuickOpen),
        ("SPC p n", NewThread),
        ("SPC q q", Quit),
        ("SPC q Q", Quit),
        ("SPC q r", Reload),
        ("SPC s p", SearchThreads),
        ("SPC s f", Filter),
        ("SPC t s", SidebarToggle),
        ("SPC t p", PanelToggle),
        ("SPC t q", ToggleSubmitMode),
        ("SPC t f", ToggleFollow),
        ("SPC t z", Zen),
        ("SPC w m m", WindowOnly),
        ("SPC w d", PaneClose),
        ("SPC TAB TAB", SwitchProject),
        ("SPC TAB .", SwitchProject),
        ("SPC TAB ]", ThreadNext),
        ("SPC TAB [", ThreadPrevious),
        ("SPC TAB n", NewThread),
        ("SPC TAB d", Archive),
    ];
    for (keys, command) in entries {
        map.bind(g, keys, cmd(*command));
    }
    let described: &[(&str, Command, &str)] = &[
        (
            "SPC s s",
            View(ViewOp::Search { forward: true }),
            "Search the focused buffer",
        ),
        (
            "SPC s b",
            View(ViewOp::Search { forward: true }),
            "Search the focused buffer",
        ),
        ("SPC s c", Ex("nohlsearch"), "Clear the search highlight"),
        ("SPC s r", Ex("marks"), "List marks"),
        ("SPC t h", Ex("set hlsearch!"), "Toggle search highlight"),
        ("SPC h x", ExPrompt("help :"), "Describe an ex command"),
    ];
    for (keys, command, desc) in described {
        map.bind_desc(g, keys, cmd(*command), desc);
    }
    for n in 1..=4u8 {
        map.bind(g, &format!("SPC {n}"), cmd(PaneFocus(n)));
    }
    for n in 1..=9u8 {
        map.bind(g, &format!("SPC TAB {n}"), cmd(Jump(n)));
    }
}

fn windows(map: &mut Keymap, prefix: &str) {
    use Command::*;
    let g = Scope::Global;
    let directions = [
        ("h", "C-h", "←", Direction::Left),
        ("j", "C-j", "↓", Direction::Down),
        ("k", "C-k", "↑", Direction::Up),
        ("l", "C-l", "→", Direction::Right),
    ];
    for (letter, control, arrow, direction) in directions {
        for keys in [letter, control, arrow] {
            map.bind(g, &format!("{prefix} {keys}"), cmd(WindowFocus(direction)));
        }
    }
    let entries: &[(&str, Command)] = &[
        ("w", FocusNext),
        ("C-w", FocusNext),
        ("W", FocusPrevious),
        ("p", WindowLast),
        ("t", WindowTop),
        ("b", WindowBottom),
        ("v", SplitOpen),
        ("s", SplitOpen),
        ("n", SplitNew),
        ("q", PaneClose),
        ("c", PaneClose),
        ("o", WindowOnly),
        ("=", WindowRestore),
        ("x", WindowSwap),
        ("r", WindowRotate),
        ("H", WindowMoveFar(true)),
        ("L", WindowMoveFar(false)),
        ("<", WindowResize(-1)),
        (">", WindowResize(1)),
    ];
    for (keys, command) in entries {
        map.bind(g, &format!("{prefix} {keys}"), cmd(*command));
    }
}

fn list_motions(map: &mut Keymap, scope: Scope) {
    use Command::*;
    let entries: &[(&str, Command)] = &[
        ("j", MoveDown),
        ("↓", MoveDown),
        ("k", MoveUp),
        ("↑", MoveUp),
        ("g g", MoveTop),
        ("Home", MoveTop),
        ("G", MoveBottom),
        ("End", MoveBottom),
        ("C-d", HalfPageDown),
        ("C-u", HalfPageUp),
        ("C-f", PageDown),
        ("C-b", PageUp),
        ("PgDn", PageDown),
        ("PgUp", PageUp),
        ("C-e", ScrollLine(1)),
        ("C-y", ScrollLine(-1)),
        ("z z", ScrollCursor(ScreenPos::Middle)),
        ("z .", ScrollCursor(ScreenPos::Middle)),
        ("z t", ScrollCursor(ScreenPos::Top)),
        ("z RET", ScrollCursor(ScreenPos::Top)),
        ("z b", ScrollCursor(ScreenPos::Bottom)),
        ("z -", ScrollCursor(ScreenPos::Bottom)),
        ("H", ViewJump(ScreenPos::Top)),
        ("M", ViewJump(ScreenPos::Middle)),
        ("L", ViewJump(ScreenPos::Bottom)),
        ("TAB", ToggleGroup),
        ("z a", ToggleGroup),
        ("z o", Expand),
        ("z O", Expand),
        ("z c", Collapse),
        ("z C", Collapse),
        ("z R", FoldAll(true)),
        ("z M", FoldAll(false)),
        ("g r", Reload),
    ];
    for (keys, command) in entries {
        map.bind(scope, keys, cmd(*command));
    }
}

fn sidebar(map: &mut Keymap) {
    use Command::*;
    let s = Scope::Sidebar;
    list_motions(map, s);
    let entries: &[(&str, Command)] = &[
        ("RET", Open),
        ("l", Expand),
        ("→", Expand),
        ("h", Collapse),
        ("←", Collapse),
        ("g o", OpenFocus),
        ("i", OpenInsert),
        ("a", OpenInsert),
        ("o", NewThread),
        ("O", NewThreadInEnvironment),
        ("R", Rename),
        ("d d", Archive),
        ("x", Archive),
        ("Del", Archive),
        ("u", UndoArchive),
        ("!", ToggleRead),
        ("+", TogglePin),
        ("y y", YankTitle),
        ("g y", YankId),
        ("g /", Filter),
        ("] u", StepUnread(true)),
        ("[ u", StepUnread(false)),
    ];
    for (keys, command) in entries {
        map.bind(s, keys, cmd(*command));
    }
    map.group(s, "d", "+archive");
    map.group(s, "y", "+yank");
}

fn timeline(map: &mut Keymap) {
    use Command::*;
    let t = Scope::Timeline;
    list_motions(map, t);
    let entries: &[(&str, Command)] = &[
        ("{", MoveUp),
        ("}", MoveDown),
        ("RET", Open),
        ("] m", NextMessage),
        ("[ m", PreviousMessage),
        ("g y", Copy),
        ("g x", OpenExternal),
        ("i", ComposerInsert(InsertAt::After)),
        ("a", ComposerInsert(InsertAt::After)),
        ("A", ComposerInsert(InsertAt::LineEnd)),
        ("o", ComposerInsert(InsertAt::Below)),
        ("O", ComposerInsert(InsertAt::Above)),
        ("I", ComposerInsert(InsertAt::LineStart)),
        ("g i", ComposerInsert(InsertAt::Last)),
    ];
    for (keys, command) in entries {
        map.bind(t, keys, cmd(*command));
    }
}

fn motions() -> Vec<(&'static str, Motion)> {
    vec![
        ("h", Motion::Left),
        ("←", Motion::Left),
        ("l", Motion::Right),
        ("→", Motion::Right),
        ("j", Motion::Down),
        ("↓", Motion::Down),
        ("k", Motion::Up),
        ("↑", Motion::Up),
        ("g j", Motion::Down),
        ("g k", Motion::Up),
        ("w", Motion::WordForward),
        ("W", Motion::BigWordForward),
        ("b", Motion::WordBackward),
        ("B", Motion::BigWordBackward),
        ("e", Motion::WordEnd),
        ("E", Motion::BigWordEnd),
        ("g e", Motion::WordEndBackward),
        ("g E", Motion::BigWordEndBackward),
        ("0", Motion::LineStart),
        ("Home", Motion::LineStart),
        ("^", Motion::FirstNonBlank),
        ("$", Motion::LineEnd),
        ("End", Motion::LineEnd),
        ("g _", Motion::LastNonBlank),
        ("g g", Motion::FirstLine),
        ("G", Motion::LastLine),
        ("}", Motion::ParagraphForward),
        ("{", Motion::ParagraphBackward),
        (")", Motion::SentenceForward),
        ("(", Motion::SentenceBackward),
        ("%", Motion::MatchPair),
        (";", Motion::RepeatFind { reverse: false }),
        (",", Motion::RepeatFind { reverse: true }),
    ]
}

fn objects() -> Vec<(&'static str, Object)> {
    vec![
        ("w", Object::Word),
        ("W", Object::BigWord),
        ("s", Object::Sentence),
        ("p", Object::Paragraph),
        ("\"", Object::Quote('"')),
        ("'", Object::Quote('\'')),
        ("`", Object::Quote('`')),
        ("(", Object::Pair('(', ')')),
        (")", Object::Pair('(', ')')),
        ("b", Object::Pair('(', ')')),
        ("[", Object::Pair('[', ']')),
        ("]", Object::Pair('[', ']')),
        ("{", Object::Pair('{', '}')),
        ("}", Object::Pair('{', '}')),
        ("B", Object::Pair('{', '}')),
        ("<", Object::Pair('<', '>')),
        (">", Object::Pair('<', '>')),
    ]
}

fn finds() -> [(&'static str, bool, bool); 4] {
    [
        ("f", true, false),
        ("F", false, false),
        ("t", true, true),
        ("T", false, true),
    ]
}

fn composer(map: &mut Keymap) {
    let c = Scope::Composer;
    for (keys, motion) in motions() {
        map.bind(c, keys, edit(EditOp::Move(motion)));
    }
    for (keys, forward, till) in finds() {
        map.bind(c, keys, Act::Char(CharAct::Find { forward, till }));
    }
    let operators = [
        ("d", "d", Operator::Delete),
        ("c", "c", Operator::Change),
        ("y", "y", Operator::Yank),
        (">", ">", Operator::Indent),
        ("<", "<", Operator::Outdent),
        ("g u", "u", Operator::Lower),
        ("g U", "U", Operator::Upper),
        ("g ~", "~", Operator::ToggleCase),
    ];
    for (prefix, double, op) in operators {
        map.bind(
            c,
            &format!("{prefix} {double}"),
            edit(EditOp::Apply(op, Target::Line)),
        );
        for (keys, motion) in motions() {
            map.bind(
                c,
                &format!("{prefix} {keys}"),
                edit(EditOp::Apply(op, Target::Motion(motion))),
            );
        }
        for (keys, forward, till) in finds() {
            map.bind(
                c,
                &format!("{prefix} {keys}"),
                Act::Char(CharAct::ApplyFind { op, forward, till }),
            );
        }
        map.bind(
            c,
            &format!("{prefix} '"),
            Act::Char(CharAct::ApplyMark { op, line: true }),
        );
        map.bind(
            c,
            &format!("{prefix} `"),
            Act::Char(CharAct::ApplyMark { op, line: false }),
        );
        for (keys, object) in objects() {
            map.bind(
                c,
                &format!("{prefix} i {keys}"),
                edit(EditOp::Apply(op, Target::Inner(object))),
            );
            map.bind(
                c,
                &format!("{prefix} a {keys}"),
                edit(EditOp::Apply(op, Target::Around(object))),
            );
        }
        map.group(c, prefix, "+operator");
        map.group(c, &format!("{prefix} i"), "+inner");
        map.group(c, &format!("{prefix} a"), "+around");
    }
    map.bind(
        c,
        "g u u",
        edit(EditOp::Apply(Operator::Lower, Target::Line)),
    );
    map.bind(
        c,
        "g U U",
        edit(EditOp::Apply(Operator::Upper, Target::Line)),
    );
    map.bind(
        c,
        "g ~ ~",
        edit(EditOp::Apply(Operator::ToggleCase, Target::Line)),
    );
    let entries: &[(&str, EditOp)] = &[
        ("i", EditOp::Insert(InsertAt::Before)),
        ("a", EditOp::Insert(InsertAt::After)),
        ("I", EditOp::Insert(InsertAt::LineStart)),
        ("A", EditOp::Insert(InsertAt::LineEnd)),
        ("o", EditOp::Insert(InsertAt::Below)),
        ("O", EditOp::Insert(InsertAt::Above)),
        ("g i", EditOp::Insert(InsertAt::Last)),
        ("x", EditOp::DeleteChar),
        ("Del", EditOp::DeleteChar),
        ("X", EditOp::DeleteCharBefore),
        ("s", EditOp::Substitute),
        ("S", EditOp::Apply(Operator::Change, Target::Line)),
        (
            "D",
            EditOp::Apply(Operator::Delete, Target::Motion(Motion::LineEnd)),
        ),
        (
            "C",
            EditOp::Apply(Operator::Change, Target::Motion(Motion::LineEnd)),
        ),
        (
            "Y",
            EditOp::Apply(Operator::Yank, Target::Motion(Motion::LineEnd)),
        ),
        ("~", EditOp::ToggleCaseChar),
        ("J", EditOp::Join { spaces: true }),
        ("g J", EditOp::Join { spaces: false }),
        ("p", EditOp::Put { before: false }),
        ("P", EditOp::Put { before: true }),
        ("u", EditOp::Undo),
        ("C-r", EditOp::Redo),
        ("U", EditOp::UndoLine),
        (".", EditOp::Repeat),
        ("R", EditOp::ReplaceMode),
        ("v", EditOp::Visual { line: false }),
        ("V", EditOp::Visual { line: true }),
        ("g v", EditOp::Reselect),
        ("C-p", EditOp::HistoryPrevious),
        ("C-n", EditOp::HistoryNext),
    ];
    for (keys, op) in entries {
        map.bind(c, keys, edit(*op));
    }
    map.bind(c, "r", Act::Char(CharAct::Replace));
    map.bind(c, "\"", Act::Char(CharAct::Register));
    map.bind(c, "m", Act::Char(CharAct::SetMark));
    map.bind(c, "'", Act::Char(CharAct::Mark { line: true }));
    map.bind(c, "`", Act::Char(CharAct::Mark { line: false }));
    map.bind(c, "RET", cmd(Command::Send));
    map.bind(c, "C-RET", cmd(Command::Send));
    map.bind(c, "M-RET", cmd(Command::Send));
}

fn visual(map: &mut Keymap) {
    let v = Scope::Visual;
    for (keys, motion) in motions() {
        map.bind(v, keys, edit(EditOp::Move(motion)));
    }
    for (keys, forward, till) in finds() {
        map.bind(v, keys, Act::Char(CharAct::Find { forward, till }));
    }
    for (keys, object) in objects() {
        map.bind(
            v,
            &format!("i {keys}"),
            edit(EditOp::VisualObject(Target::Inner(object))),
        );
        map.bind(
            v,
            &format!("a {keys}"),
            edit(EditOp::VisualObject(Target::Around(object))),
        );
    }
    map.group(v, "i", "+inner");
    map.group(v, "a", "+around");
    let entries: &[(&str, EditOp)] = &[
        ("d", EditOp::VisualApply(Operator::Delete)),
        ("x", EditOp::VisualApply(Operator::Delete)),
        ("Del", EditOp::VisualApply(Operator::Delete)),
        ("c", EditOp::VisualApply(Operator::Change)),
        ("s", EditOp::VisualApply(Operator::Change)),
        ("y", EditOp::VisualApply(Operator::Yank)),
        (">", EditOp::VisualApply(Operator::Indent)),
        ("<", EditOp::VisualApply(Operator::Outdent)),
        ("u", EditOp::VisualApply(Operator::Lower)),
        ("U", EditOp::VisualApply(Operator::Upper)),
        ("~", EditOp::VisualApply(Operator::ToggleCase)),
        ("J", EditOp::VisualJoin),
        ("p", EditOp::VisualPut),
        ("P", EditOp::VisualPut),
        ("o", EditOp::VisualSwap),
        ("v", EditOp::Visual { line: false }),
        ("V", EditOp::Visual { line: true }),
    ];
    for (keys, op) in entries {
        map.bind(v, keys, edit(*op));
    }
    map.bind(v, "r", Act::Char(CharAct::VisualReplace));
    map.bind(v, "\"", Act::Char(CharAct::Register));
    map.bind(v, "'", Act::Char(CharAct::Mark { line: true }));
    map.bind(v, "`", Act::Char(CharAct::Mark { line: false }));
}

fn insert(map: &mut Keymap) {
    use Command::*;
    let i = Scope::Insert;
    map.bind(i, "C-c C-c", cmd(Send));
    map.bind(i, "C-c C-s", cmd(Steer));
    map.bind(i, "C-c C-k", cmd(StopTurn));
    map.bind(i, "C-c C-a", cmd(AttachFile));
    map.group(i, "C-c", "+thread");
}

fn new_thread(map: &mut Keymap) {
    let n = Scope::NewThread;
    let entries: &[(&str, &str, &str)] = &[
        ("j", "j", "Next field"),
        ("↓", "↓", "Next field"),
        ("TAB", "TAB", "Next field"),
        ("k", "k", "Previous field"),
        ("↑", "↑", "Previous field"),
        ("S-TAB", "S-TAB", "Previous field"),
        ("RET", "RET", "Pick a value"),
        ("l", "RET", "Pick a value"),
        ("i", "i", "Edit the prompt"),
        ("a", "i", "Edit the prompt"),
        ("C-j", "i", "Edit the prompt"),
        ("q", "ESC", "Close the form"),
    ];
    for (keys, legacy, desc) in entries {
        map.bind_desc(n, keys, key(legacy), desc);
    }
}

fn passthrough(map: &mut Keymap, scope: Scope, entries: &[(&str, &str, &str)]) {
    for (keys, legacy, desc) in entries {
        map.bind_desc(scope, keys, key(legacy), desc);
    }
}

const NAV: &[(&str, &str, &str)] = &[
    ("j", "j", "Move down"),
    ("↓", "↓", "Move down"),
    ("k", "k", "Move up"),
    ("↑", "↑", "Move up"),
    ("g g", "g", "Go to the first line"),
    ("Home", "Home", "Go to the first line"),
    ("G", "G", "Go to the last line"),
    ("End", "End", "Go to the last line"),
    ("PgDn", "PgDn", "Page down"),
    ("PgUp", "PgUp", "Page up"),
    ("C-f", "C-f", "Page down"),
    ("C-b", "C-b", "Page up"),
    ("C-d", "C-d", "Half page down"),
    ("C-u", "C-u", "Half page up"),
];

fn panel(map: &mut Keymap) {
    let p = Scope::Panel;
    passthrough(map, p, NAV);
    passthrough(
        map,
        p,
        &[
            ("g r", "r", "Reload the panel"),
            ("q", "ESC", "Back / leave the panel"),
            ("RET", "RET", "Open / fold"),
        ],
    );
    passthrough(
        map,
        Scope::PanelInfo,
        &[
            ("g f", "p", "Preview the file"),
            ("e", "e", "Open the file in $EDITOR"),
            ("y y", "y", "Yank the SHA, path or URL"),
            ("g x", "o", "Open the link"),
        ],
    );
    passthrough(
        map,
        Scope::PanelDiff,
        &[
            ("TAB", "RET", "Fold / unfold the file"),
            ("z a", "RET", "Fold / unfold the file"),
            ("z h", "h", "Scroll left"),
            ("z l", "l", "Scroll right"),
            ("0", "0", "Back to the first column"),
            ("] c", "n", "Next hunk"),
            ("[ c", "N", "Previous hunk"),
            ("] f", "]", "Next file"),
            ("[ f", "[", "Previous file"),
            ("s", "s", "Next scope"),
            ("S", "S", "Previous scope"),
            ("g f", "p", "Preview the file at the cursor line"),
            ("e", "e", "Open the file in $EDITOR"),
            ("y y", "y", "Yank the line or the path"),
        ],
    );
    map.bind(Scope::PanelDiff, "z R", cmd(Command::FoldAll(true)));
    map.bind(Scope::PanelDiff, "z M", cmd(Command::FoldAll(false)));
    passthrough(
        map,
        Scope::PanelFile,
        &[
            ("z h", "h", "Scroll left"),
            ("z l", "l", "Scroll right"),
            ("0", "0", "Back to the first column"),
            ("e", "e", "Open the file in $EDITOR"),
            ("g y", "y", "Yank the path"),
        ],
    );
    passthrough(
        map,
        Scope::PanelTerminal,
        &[
            ("i", "i", "Type into the terminal"),
            ("a", "i", "Type into the terminal"),
            ("I", "i", "Type into the terminal"),
            ("A", "i", "Type into the terminal"),
            ("] t", "]", "Next terminal"),
            ("[ t", "[", "Previous terminal"),
            ("o", "n", "Start a new terminal"),
            ("d d", "x", "Close the terminal"),
            ("x", "x", "Close the terminal"),
            ("y y", "y", "Yank the visible screen"),
        ],
    );
}

fn views(map: &mut Keymap) {
    let n = Scope::Notifications;
    passthrough(map, n, NAV);
    passthrough(
        map,
        n,
        &[
            ("RET", "RET", "Open the thread"),
            ("d d", "x", "Delete the notification"),
            ("x", "x", "Delete the notification"),
            ("Del", "x", "Delete the notification"),
            ("C", "C", "Clear all"),
            ("q", "ESC", "Close"),
        ],
    );
    let pane = Scope::Pane;
    passthrough(map, pane, NAV);
    passthrough(
        map,
        pane,
        &[
            ("z h", "h", "Scroll left"),
            ("z l", "l", "Scroll right"),
            ("0", "0", "Back to the first column"),
            ("y y", "y", "Yank the output or path"),
            ("g y", "y", "Yank the output or path"),
            ("g x", "o", "Open the previewed file"),
            ("q", "ESC", "Close"),
            ("RET", "ESC", "Close"),
        ],
    );
    let s = Scope::Settings;
    passthrough(map, s, NAV);
    passthrough(
        map,
        s,
        &[
            ("RET", "RET", "Open / edit"),
            ("l", "l", "Open"),
            ("→", "→", "Open"),
            ("h", "h", "Back to the section list"),
            ("←", "←", "Back to the section list"),
            ("TAB", "TAB", "Switch list / section"),
            ("S-TAB", "S-TAB", "Switch list / section"),
            ("~", "SPC", "Toggle"),
            ("R", "r", "Rename"),
            ("d d", "D", "Delete"),
            ("x", "D", "Delete"),
            ("Del", "Del", "Delete"),
            ("u", "u", "Unarchive"),
            ("g d", "d", "Make default provider"),
            ("z c", "c", "Toggle collapsing finished turns"),
            ("t", "t", "Cycle archived thread type"),
            ("p", "p", "Cycle archived project"),
            ("e", "e", "Edit"),
            ("g /", "/", "Narrow to a filter"),
            ("y y", "y", "Yank"),
            ("g r", "C-r", "Reload the section"),
            ("q", "q", "Close settings"),
        ],
    );
    let a = Scope::Automations;
    passthrough(map, a, NAV);
    passthrough(
        map,
        a,
        &[
            ("RET", "RET", "Open"),
            ("l", "l", "Open"),
            ("h", "h", "Back"),
            ("←", "←", "Back"),
            ("o", "n", "New automation"),
            ("~", "SPC", "Pause / resume"),
            ("r", "r", "Run now"),
            ("e", "e", "Edit"),
            ("d d", "d", "Delete"),
            ("x", "d", "Delete"),
            ("Del", "Del", "Delete"),
            ("g /", "/", "Narrow to a filter"),
            ("g r", "R", "Reload"),
            ("g m", "m", "Load more runs"),
            ("q", "q", "Close"),
        ],
    );
}
