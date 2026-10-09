mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use bb_client::{BbClient, ServerEndpoint, TerminalConnectionState, TerminalEvent};
use bb_store::{NoRealtime, Store, StoreConfig};
use bb_tui::app::{Action, AppState, Effect, Focus, ServerInfo, update};
use bb_tui::panel::PanelTab;
use bb_tui::platform::{RawEvent, RawInput};
use bb_tui::runtime::run;
use bb_tui::terminal::{TerminalLoaded, TerminalRequest, TerminalScope};
use bb_tui::terminal_input::{DETACH_BYTE, RESET_MODES};
use bb_wire::terminal::{
    TerminalCloseMode, TerminalCreateTarget, TerminalOutputChunk, TerminalServerMessage,
    TerminalSession, TerminalSessionStatus,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use futures::stream;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;
use support::ws::{MockTerminalServer, attached, output, session_updated};
use support::*;
use tokio::sync::mpsc;

const ENV: &str = "env_1";

fn sidebar_with_environment() -> bb_wire::project::SidebarBootstrapResponse {
    let mut sidebar = sample_sidebar();
    for entry in &mut sidebar.projects[0].threads {
        if entry.thread.id == THREAD {
            entry.thread.environment_id = Some(ENV.into());
        }
    }
    sidebar
}

fn scope() -> TerminalScope {
    TerminalScope {
        thread_id: THREAD.into(),
        environment_id: Some(ENV.into()),
    }
}

fn session(id: &str, status: TerminalSessionStatus) -> TerminalSession {
    terminal_session(id, Some(THREAD), None, status)
}

fn opened_state(read_only: bool) -> AppState {
    let mut state = loaded_state(sidebar_with_environment());
    state.read_only = read_only;
    state.open(THREAD.into());
    state
}

fn terminal_effects(effects: &[Effect]) -> Vec<TerminalRequest> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Terminal(request) => Some(request.clone()),
            _ => None,
        })
        .collect()
}

fn load_sessions(state: &mut AppState, sessions: Vec<TerminalSession>) -> Vec<TerminalRequest> {
    terminal_effects(&update(
        state,
        Action::TerminalLoaded(TerminalLoaded::List {
            scope: scope(),
            result: Ok(sessions),
        }),
    ))
}

fn event(state: &mut AppState, id: &str, link: u64, event: TerminalEvent) -> Vec<TerminalRequest> {
    terminal_effects(&update(
        state,
        Action::TerminalEvent {
            terminal_id: id.into(),
            link,
            event,
        },
    ))
}

fn out(state: &mut AppState, id: &str, link: u64, seq: u64, bytes: &[u8]) -> Vec<TerminalRequest> {
    event(
        state,
        id,
        link,
        TerminalEvent::Message(TerminalServerMessage::Output {
            chunk: TerminalOutputChunk {
                seq,
                data_base64: STANDARD.encode(bytes),
            },
        }),
    )
}

fn attach(state: &mut AppState, id: &str, link: u64) {
    event(
        state,
        id,
        link,
        TerminalEvent::State(TerminalConnectionState::Attached),
    );
    event(
        state,
        id,
        link,
        TerminalEvent::Message(TerminalServerMessage::Attached {
            session: session(id, TerminalSessionStatus::Running),
            replay_start_seq: 0,
            next_seq: 0,
        }),
    );
}

fn key(state: &mut AppState, code: KeyCode, modifiers: KeyModifiers) -> Vec<TerminalRequest> {
    terminal_effects(&update(state, Action::Key(KeyEvent::new(code, modifiers))))
}

fn type_key(state: &mut AppState, code: KeyCode) -> Vec<TerminalRequest> {
    key(state, code, KeyModifiers::NONE)
}

fn seq_keys(state: &mut AppState, text: &str) -> Vec<TerminalRequest> {
    terminal_effects(&keys(state, text))
}

fn inputs(requests: &[TerminalRequest]) -> Vec<Vec<u8>> {
    requests
        .iter()
        .filter_map(|request| match request {
            TerminalRequest::Input { bytes, .. } => Some(bytes.clone()),
            _ => None,
        })
        .collect()
}

fn tick(state: &mut AppState) -> Vec<TerminalRequest> {
    terminal_effects(&update(
        state,
        Action::Tick {
            now: Instant::now(),
            now_ms: NOW_MS,
        },
    ))
}

fn connected_state(read_only: bool) -> AppState {
    let mut state = opened_state(read_only);
    seq_keys(&mut state, "SPC o t");
    load_sessions(
        &mut state,
        vec![session("term_1", TerminalSessionStatus::Running)],
    );
    attach(&mut state, "term_1", 1);
    state
}

#[test]
fn opening_the_terminal_tab_lists_thread_and_environment_terminals_and_attaches() {
    let mut state = opened_state(false);
    let requests = seq_keys(&mut state, "SPC o t");
    assert_eq!(requests, vec![TerminalRequest::List { scope: scope() }]);
    assert_eq!(state.panel.tab, PanelTab::Terminal);
    assert_eq!(state.focus, Focus::Panel);
    assert_eq!(
        scope()
            .queries()
            .iter()
            .map(|query| (query.thread_id.clone(), query.environment_id.clone()))
            .collect::<Vec<_>>(),
        vec![(Some(THREAD.into()), None), (None, Some(ENV.into()))]
    );
    let text = render_lines(&state, 120, 30).join("\n");
    assert!(text.contains("Loading terminals"), "{text}");

    let requests = load_sessions(
        &mut state,
        vec![
            session("term_b", TerminalSessionStatus::Running),
            session("term_gone", TerminalSessionStatus::Exited),
            terminal_session("term_env", None, Some(ENV), TerminalSessionStatus::Running),
        ],
    );
    let viewport = state.terminals.viewport.get().expect("viewport recorded");
    assert_eq!(
        requests,
        vec![TerminalRequest::Connect {
            terminal_id: "term_b".into(),
            link: 1,
            since_seq: 0,
            size: Some(viewport),
            visible: true,
        }]
    );
    assert_eq!(state.terminals.session_ids(), ["term_b", "term_env"]);
    attach(&mut state, "term_b", 1);
    let acks = out(
        &mut state,
        "term_b",
        1,
        0,
        b"hello \x1b[31mred\x1b[0m \x1b[1;4mbold",
    );
    assert_eq!(
        acks,
        vec![TerminalRequest::Ack {
            terminal_id: "term_b".into(),
            link: 1,
            next_seq: 1,
        }]
    );
    assert!(out(&mut state, "term_b", 1, 0, b"dup").is_empty());

    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal
        .draw(|frame| bb_tui::ui::render(&state, frame))
        .unwrap();
    let text = buffer_lines(&terminal).join("\n");
    assert!(text.contains("hello red bold"), "{text}");
    assert!(text.contains("shell term_b"), "{text}");
    assert!(text.contains("shell term_env"), "{text}");
    assert!(!text.contains("term_gone"), "{text}");
    let buffer = terminal.backend().buffer();
    let (x, y) = (0..buffer.area.height)
        .find_map(|y| {
            (0..buffer.area.width)
                .find(|x| {
                    buffer[(*x, y)].symbol() == "r"
                        && buffer[(*x + 1, y)].symbol() == "e"
                        && buffer[(*x + 2, y)].symbol() == "d"
                })
                .map(|x| (x, y))
        })
        .expect("red text");
    assert_eq!(buffer[(x, y)].fg, Color::Indexed(1));
    let bold = &buffer[(x + 4, y)];
    assert!(bold.modifier.contains(ratatui::style::Modifier::BOLD));
    assert!(bold.modifier.contains(ratatui::style::Modifier::UNDERLINED));
}

#[test]
fn typing_is_encoded_for_the_remote_terminal_until_the_escape_chord() {
    let mut state = connected_state(false);
    assert!(state.terminal_input_active());
    let mut sent = Vec::new();
    for code in [
        KeyCode::Char('l'),
        KeyCode::Char('s'),
        KeyCode::Enter,
        KeyCode::Up,
    ] {
        sent.extend(inputs(&type_key(&mut state, code)));
    }
    sent.extend(inputs(&key(
        &mut state,
        KeyCode::Char('c'),
        KeyModifiers::CONTROL,
    )));
    sent.extend(inputs(&key(
        &mut state,
        KeyCode::Char('k'),
        KeyModifiers::CONTROL,
    )));
    assert_eq!(
        sent,
        vec![
            b"l".to_vec(),
            b"s".to_vec(),
            b"\r".to_vec(),
            b"\x1b[A".to_vec(),
            vec![0x03],
            vec![0x0b]
        ]
    );
    assert!(state.palette.is_none());

    out(&mut state, "term_1", 1, 0, b"\x1b[?1h\x1b[?2004h");
    assert_eq!(
        inputs(&type_key(&mut state, KeyCode::Up)),
        vec![b"\x1bOA".to_vec()]
    );
    let pasted = terminal_effects(&update(&mut state, Action::Paste("a\nb".into())));
    assert_eq!(inputs(&pasted), vec![b"\x1b[200~a\rb\x1b[201~".to_vec()]);

    assert!(key(&mut state, KeyCode::Char('\\'), KeyModifiers::CONTROL).is_empty());
    assert!(!state.terminal_input_active());
    assert!(inputs(&type_key(&mut state, KeyCode::Char('j'))).is_empty());
    let text = render_lines(&state, 120, 30).join("\n");
    assert!(text.contains("i type"), "{text}");
    type_key(&mut state, KeyCode::Enter);
    assert!(state.terminal_input_active());
    assert!(key(&mut state, KeyCode::Char('4'), KeyModifiers::CONTROL).is_empty());
    assert!(!state.terminal_input_active());
}

#[test]
fn viewport_changes_are_sent_as_resizes_and_the_screen_follows_the_session_size() {
    let mut state = connected_state(false);
    render_lines(&state, 120, 30);
    let (cols, rows) = state.terminals.viewport.get().unwrap();
    assert_eq!(
        tick(&mut state),
        vec![TerminalRequest::Resize {
            terminal_id: "term_1".into(),
            link: 1,
            cols,
            rows,
        }]
    );
    assert!(tick(&mut state).is_empty());
    render_lines(&state, 160, 40);
    let (wide_cols, tall_rows) = state.terminals.viewport.get().unwrap();
    assert!(wide_cols > cols && tall_rows > rows);
    assert_eq!(
        tick(&mut state),
        vec![TerminalRequest::Resize {
            terminal_id: "term_1".into(),
            link: 1,
            cols: wide_cols,
            rows: tall_rows,
        }]
    );
    assert!(tick(&mut state).is_empty());
    let screen = |state: &AppState| {
        state
            .terminals
            .active_view()
            .unwrap()
            .screen
            .screen()
            .size()
    };
    assert_eq!(screen(&state), (24, 80));
    event(
        &mut state,
        "term_1",
        1,
        TerminalEvent::Message(TerminalServerMessage::SessionUpdated {
            session: TerminalSession {
                cols: wide_cols,
                rows: tall_rows,
                ..session("term_1", TerminalSessionStatus::Running)
            },
        }),
    );
    assert_eq!(screen(&state), (tall_rows, wide_cols));
}

#[test]
fn read_only_mode_watches_terminals_but_never_types_resizes_creates_or_closes() {
    let mut state = connected_state(true);
    render_lines(&state, 120, 30);
    assert!(tick(&mut state).is_empty());
    assert!(type_key(&mut state, KeyCode::Char('l')).is_empty());
    assert!(
        state
            .toasts
            .iter()
            .any(|toast| toast.message == "Read-only mode: cannot type into a terminal")
    );
    key(&mut state, KeyCode::Char('\\'), KeyModifiers::CONTROL);
    assert!(type_key(&mut state, KeyCode::Char('o')).is_empty());
    assert!(!state.terminals.creating);
    type_key(&mut state, KeyCode::Char('x'));
    assert!(type_key(&mut state, KeyCode::Char('y')).is_empty());
    let messages: Vec<_> = state
        .toasts
        .iter()
        .map(|toast| toast.message.clone())
        .collect();
    assert!(
        messages.contains(&"Read-only mode: cannot create a terminal".to_owned()),
        "{messages:?}"
    );
    assert!(
        messages.contains(&"Read-only mode: cannot close a terminal".to_owned()),
        "{messages:?}"
    );
    out(&mut state, "term_1", 1, 0, b"still streaming");
    assert!(
        render_lines(&state, 120, 30)
            .join("\n")
            .contains("still streaming")
    );
}

#[test]
fn reconnect_gaps_reset_the_screen_and_stale_links_are_ignored() {
    let mut state = connected_state(false);
    out(&mut state, "term_1", 1, 0, b"before gap");
    event(
        &mut state,
        "term_1",
        1,
        TerminalEvent::State(TerminalConnectionState::Reconnecting),
    );
    assert!(
        render_lines(&state, 120, 30)
            .join("\n")
            .contains("Reconnecting")
    );
    event(
        &mut state,
        "term_1",
        1,
        TerminalEvent::State(TerminalConnectionState::Attached),
    );
    event(
        &mut state,
        "term_1",
        1,
        TerminalEvent::SequenceGap {
            expected: 1,
            received: 7,
        },
    );
    let view = state.terminals.active_view().unwrap();
    assert!(!view.screen.screen().contents().contains("before gap"));
    assert_eq!(view.next_seq, 7);
    assert_eq!(
        view.notice.as_deref(),
        Some("Some terminal output was unavailable after reconnect")
    );
    assert_eq!(out(&mut state, "term_1", 1, 7, b"after").len(), 1);
    assert!(out(&mut state, "term_1", 99, 8, b"stale").is_empty());
    assert!(
        !state
            .terminals
            .active_view()
            .unwrap()
            .screen
            .screen()
            .contents()
            .contains("stale")
    );

    event(
        &mut state,
        "term_1",
        1,
        TerminalEvent::Message(TerminalServerMessage::Exited {
            session: TerminalSession {
                exit_code: Some(3),
                ..session("term_1", TerminalSessionStatus::Exited)
            },
        }),
    );
    assert!(!state.terminal_input_active());
    let text = render_lines(&state, 120, 30).join("\n");
    assert!(text.contains("Terminal exited with code 3"), "{text}");
    type_key(&mut state, KeyCode::Enter);
    assert!(!state.terminals.input);
}

#[test]
fn scrollback_pages_through_history_and_typing_returns_to_the_live_screen() {
    let mut state = connected_state(false);
    let lines: String = (1..=60).map(|n| format!("line {n}\r\n")).collect();
    out(&mut state, "term_1", 1, 0, lines.as_bytes());
    key(&mut state, KeyCode::PageUp, KeyModifiers::SHIFT);
    assert!(state.terminals.active_view().unwrap().scrollback() > 0);
    let text = render_lines(&state, 120, 30).join("\n");
    assert!(text.contains("line 20"), "{text}");
    assert!(text.contains("↑"), "{text}");
    type_key(&mut state, KeyCode::Char('x'));
    assert_eq!(state.terminals.active_view().unwrap().scrollback(), 0);
    key(&mut state, KeyCode::Char('\\'), KeyModifiers::CONTROL);
    seq_keys(&mut state, "g g");
    let top = state.terminals.active_view().unwrap().scrollback();
    assert!(top >= 37, "{top}");
    type_key(&mut state, KeyCode::Char('G'));
    assert_eq!(state.terminals.active_view().unwrap().scrollback(), 0);
}

#[test]
fn new_close_and_switch_terminals_reconnect_with_replay_offsets() {
    let mut state = connected_state(false);
    render_lines(&state, 120, 30);
    key(&mut state, KeyCode::Char('\\'), KeyModifiers::CONTROL);
    let create = type_key(&mut state, KeyCode::Char('o'));
    let viewport = state.terminals.viewport.get().unwrap();
    let [TerminalRequest::Create { request, .. }] = create.as_slice() else {
        panic!("{create:?}");
    };
    assert_eq!((request.cols, request.rows), viewport);
    assert_eq!(
        request.target,
        TerminalCreateTarget::Thread {
            thread_id: THREAD.into()
        }
    );
    assert!(
        type_key(&mut state, KeyCode::Char('o')).is_empty(),
        "one creation at a time"
    );
    out(&mut state, "term_1", 1, 0, b"one");
    out(&mut state, "term_1", 1, 1, b"two");
    let requests = terminal_effects(&update(
        &mut state,
        Action::TerminalLoaded(TerminalLoaded::Created {
            scope: scope(),
            result: Ok(session("term_2", TerminalSessionStatus::Running)),
        }),
    ));
    assert_eq!(
        requests,
        vec![
            TerminalRequest::Disconnect {
                terminal_id: "term_1".into(),
                link: 1
            },
            TerminalRequest::Connect {
                terminal_id: "term_2".into(),
                link: 2,
                since_seq: 0,
                size: Some(viewport),
                visible: true,
            },
        ]
    );
    assert!(state.terminals.input);
    key(&mut state, KeyCode::Char('\\'), KeyModifiers::CONTROL);
    let back = seq_keys(&mut state, "[ t");
    assert_eq!(
        back[1],
        TerminalRequest::Connect {
            terminal_id: "term_1".into(),
            link: 3,
            since_seq: 2,
            size: Some(viewport),
            visible: true,
        }
    );
    assert!(
        state
            .terminals
            .active_view()
            .unwrap()
            .screen
            .screen()
            .contents()
            .contains("onetwo")
    );

    assert!(type_key(&mut state, KeyCode::Char('x')).is_empty());
    assert!(
        render_lines(&state, 120, 30)
            .join("\n")
            .contains("Close shell term_1?")
    );
    assert!(type_key(&mut state, KeyCode::Char('q')).is_empty());
    assert!(state.terminals.confirm_close.is_none());
    type_key(&mut state, KeyCode::Char('x'));
    let close = type_key(&mut state, KeyCode::Char('y'));
    let [
        TerminalRequest::Close {
            terminal_id,
            request,
        },
    ] = close.as_slice()
    else {
        panic!("{close:?}");
    };
    assert_eq!(terminal_id, "term_1");
    assert_eq!(request.mode, TerminalCloseMode::Force);
    let closed = terminal_effects(&update(
        &mut state,
        Action::TerminalLoaded(TerminalLoaded::Closed {
            terminal_id: "term_1".into(),
            result: Ok(session("term_1", TerminalSessionStatus::Exited)),
        }),
    ));
    assert!(closed.contains(&TerminalRequest::Disconnect {
        terminal_id: "term_1".into(),
        link: 3
    }));
    assert_eq!(state.terminals.session_ids(), ["term_2"]);
    assert_eq!(state.terminals.active.as_deref(), Some("term_2"));
}

#[test]
fn leaving_the_tab_hides_the_terminal_and_another_thread_disconnects_it() {
    let mut state = connected_state(false);
    key(&mut state, KeyCode::Char('\\'), KeyModifiers::CONTROL);
    let hidden = seq_keys(&mut state, "1 g t");
    assert_eq!(
        hidden,
        vec![TerminalRequest::Visible {
            terminal_id: "term_1".into(),
            link: 1,
            visible: false
        }]
    );
    let shown = seq_keys(&mut state, "4 g t");
    assert!(shown.contains(&TerminalRequest::Visible {
        terminal_id: "term_1".into(),
        link: 1,
        visible: true
    }));
    let effects = terminal_effects(&state.open("thr_other".into()));
    assert!(effects.contains(&TerminalRequest::Disconnect {
        terminal_id: "term_1".into(),
        link: 1
    }));
    assert!(state.terminals.views.is_empty());
}

fn base_state() -> AppState {
    AppState::new(
        ServerInfo {
            url: "http://fake/".into(),
            profile: None,
        },
        Instant::now(),
        NOW_MS,
    )
}

async fn wait_until(predicate: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("condition");
}

fn send_key(tx: &mpsc::UnboundedSender<Action>, code: KeyCode, modifiers: KeyModifiers) {
    tx.send(Action::Key(KeyEvent::new(code, modifiers)))
        .unwrap();
}

#[tokio::test]
async fn runtime_streams_a_real_terminal_socket_with_acks_input_reconnect_and_fullscreen() {
    let mut server = MockTerminalServer::start().await;
    let backend = FakeBackend::new(sidebar_with_environment());
    let mut api = MockApi::new(backend.clone());
    {
        let api = Arc::get_mut(&mut api).unwrap();
        api.terminal_client =
            Some(BbClient::local(ServerEndpoint::parse(&server.url).unwrap()).unwrap());
        api.terminal_sessions
            .lock()
            .unwrap()
            .push(session("term_1", TerminalSessionStatus::Running));
    }
    let (raw_tx, raw_rx) = mpsc::unbounded_channel();
    let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_flag = stopped.clone();
    let platform = Arc::new(MockPlatform {
        screen_size: Some((132, 40)),
        ..MockPlatform::default()
    });
    *platform.raw_input.lock().unwrap() = Some(RawInput::new(raw_rx, move || {
        stop_flag.store(true, std::sync::atomic::Ordering::SeqCst);
    }));
    let store = Store::spawn(backend.clone(), NoRealtime, None, StoreConfig::default());
    let (tx, mut rx) = mpsc::unbounded_channel::<Action>();
    let events = Box::pin(stream::poll_fn(move |cx| rx.poll_recv(cx)));
    let task_api = api.clone();
    let task_platform = platform.clone();
    let handle = tokio::spawn(async move {
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        let mut state = base_state();
        run(
            &mut terminal,
            &mut state,
            store,
            task_api,
            task_platform,
            events,
        )
        .await
        .expect("run");
        (terminal, state)
    });

    let sidebar_api = api.clone();
    wait_until(move || *sidebar_api.backend.sidebar_fetches.lock().unwrap() > 0).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    send_key(&tx, KeyCode::Enter, KeyModifiers::NONE);
    for ch in [' ', 'o', 't'] {
        send_key(&tx, KeyCode::Char(ch), KeyModifiers::NONE);
    }

    let resize = server.frame_where(|frame| frame.kind() == "resize").await;
    let (cols, rows) = (
        resize.json["cols"].as_u64().unwrap(),
        resize.json["rows"].as_u64().unwrap(),
    );
    assert!(cols > 20 && rows > 5, "{resize:?}");
    assert_eq!(server.queries(), ["sinceSeq=0&outputAcks=1"]);
    server.send(attached("term_1", 0, cols as u16, rows as u16));
    server.send(output(0, b"PROMPT$ "));
    let ack = server.frame_where(|frame| frame.kind() == "ack").await;
    assert_eq!(ack.json["nextSeq"], 1);

    for ch in ['l', 's'] {
        send_key(&tx, KeyCode::Char(ch), KeyModifiers::NONE);
    }
    send_key(&tx, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(server.input_until(b"\r").await, b"ls\r");
    server.send(output(1, b"ls\r\nfile.txt\r\nPROMPT$ "));
    assert_eq!(
        server.frame_where(|frame| frame.kind() == "ack").await.json["nextSeq"],
        2
    );

    server.drop_latest();
    let mut deadline = 0;
    while server.connection_count() < 2 && deadline < 500 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        deadline += 1;
    }
    assert_eq!(
        server.queries().last().map(String::as_str),
        Some("sinceSeq=2&outputAcks=1")
    );
    server
        .frame_where(|frame| frame.connection == 1 && frame.kind() == "resize")
        .await;
    server.send(attached("term_1", 2, cols as u16, rows as u16));
    server.send(output(2, b"echo replayed"));
    assert_eq!(
        server.frame_where(|frame| frame.kind() == "ack").await.json["nextSeq"],
        3
    );

    send_key(&tx, KeyCode::Char('\\'), KeyModifiers::CONTROL);
    for ch in [' ', 'o', 'f'] {
        send_key(&tx, KeyCode::Char(ch), KeyModifiers::NONE);
    }
    let resize = server.frame_where(|frame| frame.kind() == "resize").await;
    assert_eq!(
        (resize.json["cols"].as_u64(), resize.json["rows"].as_u64()),
        (Some(132), Some(40))
    );
    server.send(session_updated("term_1", 132, 40));
    let painted = platform.clone();
    wait_until(move || {
        String::from_utf8_lossy(&painted.raw_output.lock().unwrap()).contains("echo replayed")
    })
    .await;
    assert_eq!(*platform.screen_events.lock().unwrap(), ["suspend"]);
    raw_tx.send(RawEvent::Bytes(b"pwd\r".to_vec())).unwrap();
    assert_eq!(server.input_until(b"\r").await, b"pwd\r");
    server.send(output(3, b"\r\n/w/fullscreen"));
    let streamed = platform.clone();
    wait_until(move || {
        String::from_utf8_lossy(&streamed.raw_output.lock().unwrap()).contains("/w/fullscreen")
    })
    .await;
    raw_tx
        .send(RawEvent::Resize {
            cols: 100,
            rows: 30,
        })
        .unwrap();
    let resize = server.frame_where(|frame| frame.kind() == "resize").await;
    assert_eq!(
        (resize.json["cols"].as_u64(), resize.json["rows"].as_u64()),
        (Some(100), Some(30))
    );
    raw_tx
        .send(RawEvent::Bytes(vec![b'q', DETACH_BYTE]))
        .unwrap();
    assert_eq!(server.input_until(b"q").await, b"q");
    let resumed = platform.clone();
    wait_until(move || resumed.screen_events.lock().unwrap().len() == 2).await;
    assert!(stopped.load(std::sync::atomic::Ordering::SeqCst));
    assert!(platform.raw_output.lock().unwrap().ends_with(RESET_MODES));

    tokio::time::sleep(Duration::from_millis(300)).await;
    send_key(&tx, KeyCode::Char('\\'), KeyModifiers::CONTROL);
    for ch in [' ', 'q', 'q'] {
        send_key(&tx, KeyCode::Char(ch), KeyModifiers::NONE);
    }
    let (terminal, state) = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("loop exits")
        .expect("join");
    let text = buffer_lines(&terminal).join("\n");
    assert!(text.contains("file.txt"), "{text}");
    assert!(text.contains("echo replayed"), "{text}");
    assert!(text.contains("/w/fullscreen"), "{text}");
    assert!(state.terminals.fullscreen.is_none());
    assert!(
        state
            .toasts
            .iter()
            .any(|toast| toast.message == "Detached from shell term_1"),
        "{:?}",
        state.toasts
    );
    assert_eq!(
        api.terminal_connects.lock().unwrap().clone(),
        [("term_1".to_owned(), 0)]
    );
    assert_eq!(api.terminal_queries.lock().unwrap().len(), 2);
}

#[test]
fn terminal_scrollback_is_searchable_and_yankable_in_normal_state() {
    let mut state = connected_state(false);
    let lines: String = (1..=60).map(|n| format!("line {n}\r\n")).collect();
    out(&mut state, "term_1", 1, 0, lines.as_bytes());
    render_lines(&state, 120, 30);
    key(&mut state, KeyCode::Char('\\'), KeyModifiers::CONTROL);
    seq_keys(&mut state, "g g");
    assert!(state.terminals.active_view().unwrap().scrollback() > 0);
    let effects = update(&mut state, Action::Key(KeyEvent::from(KeyCode::Char('y'))));
    assert!(effects.is_empty());
    let effects = keys(&mut state, "y");
    assert_eq!(effects, [Effect::Copy("line 1\n".into())]);
    keys(&mut state, "/ l i n e SPC 3 0 RET");
    let effects = keys(&mut state, "y y");
    assert_eq!(effects, [Effect::Copy("line 30\n".into())]);
    let effects = keys(&mut state, "SPC b y");
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::Copy(text) if text.contains("line"))),
        "{effects:?}"
    );
    seq_keys(&mut state, "G");
    assert_eq!(state.terminals.active_view().unwrap().scrollback(), 0);
}
