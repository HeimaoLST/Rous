use super::*;
use crate::{
    client::{
        SessionMetadata, SessionSummary,
        history::{History, HistoryEntry, TaskStatus},
        types::{InputItem, InputMessageContent, MessageRole},
    },
    frontend::tui::sessions::Sessions,
    input::ImageAttachment,
    session::{SessionCommand, SessionEvent},
};

pub(super) fn snapshot(title: &str) -> (SessionMetadata, History) {
    let id = crate::client::history::new_id().unwrap();
    let metadata = crate::client::tests::metadata(&id, title);
    let history = History {
        history: vec![HistoryEntry::Messages {
            items: vec![
                InputItem::Message {
                    role: MessageRole::User,
                    content: InputMessageContent::Text("original input".into()),
                },
                InputItem::Reasoning {
                    content: vec![crate::client::types::ReasoningText::new("inspect first")],
                    provider_data: None,
                },
                InputItem::Message {
                    role: MessageRole::Assistant,
                    content: InputMessageContent::Text("original answer".into()),
                },
            ],
        }],
        status: TaskStatus::Unknown,
    };
    (metadata, history)
}
pub(super) fn task_snapshot(title: &str) -> (SessionMetadata, History) {
    let (metadata, mut history) = snapshot(title);
    history.history = vec![
        HistoryEntry::TaskStarted {
            task_id: "task".into(),
            message_id: Some("input".into()),
            configuration: None,
        },
        HistoryEntry::Messages {
            items: vec![
                InputItem::Message {
                    role: MessageRole::User,
                    content: InputMessageContent::Text("task question".into()),
                },
                InputItem::Reasoning {
                    content: vec![crate::client::types::ReasoningText::new("task thought")],
                    provider_data: None,
                },
                InputItem::Message {
                    role: MessageRole::Assistant,
                    content: InputMessageContent::Text("task answer".into()),
                },
            ],
        },
        HistoryEntry::TaskFinished {
            task_id: "task".into(),
            status: TaskStatus::Completed,
            error: None,
        },
    ];
    history.status = TaskStatus::Completed;
    (metadata, history)
}

pub(super) fn idle_state(id: &str) -> crate::client::SessionState {
    crate::client::SessionState {
        session_id: id.to_owned(),
        metadata: None,
        loaded: true,
        state: "idle".into(),
        unavailable: None,
        active_task: None,
        status: "idle".to_owned(),
        effective_config: None,
        config_revision: None,
        storage_schema: None,
    }
}

pub(super) fn opened(metadata: SessionMetadata, replay: History) -> SessionEvent {
    let state = idle_state(&metadata.id);
    SessionEvent::Opened {
        metadata,
        replay,
        context: AgentEvent::ContextUpdated {
            model: "test-model".into(),
            context_tokens: Some(1000),
            context_window: Some(1_000_000),
            input_limit: Some(991_808),
        },
        state,
        stream: None,
    }
}

#[test]
fn switching_restores_drafts_images_reading_position_and_thought_expansion() {
    let (a, mut a_history) = snapshot("Session A");
    let (b, b_history) = snapshot("Session B");
    let mut sessions = Sessions::default();
    let mut app = App::new();
    let mut editor = Editor::new();
    sessions.apply_event(&mut app, &mut editor, opened(a.clone(), a_history.clone()));
    assert!(!app.running);
    assert!(matches!(
        app.records[1],
        Record::Reasoning {
            expanded: false,
            ..
        }
    ));
    app.selected = 1;
    app.toggle_selected();
    app.focus = Focus::Transcript;
    app.follow = false;
    app.anchor = Some(Anchor {
        record: 1,
        offset: 0,
    });
    editor.set_draft(crate::input::UserInput {
        context: Vec::new(),
        text: "unsent A draft".into(),
        images: vec![ImageAttachment::new("a.png", "data:image/png;base64,AA")],
    });
    sessions.apply_event(&mut app, &mut editor, opened(b.clone(), b_history));
    assert!(editor.is_empty());
    assert_eq!(app.session.as_ref().unwrap().id, b.id);
    assert!(
        !editor.undo(),
        "a switched-to session starts with no undo history"
    );
    editor.insert_str("unsent B draft");
    sessions.apply_event(
        &mut app,
        &mut editor,
        SessionEvent::Agent {
            session_id: a.id.clone(),
            event: AgentEvent::ResponseText {
                item_id: "late".into(),
                text: "must not appear in B".into(),
            },
        },
    );
    assert!(!format!("{:?}", app.records).contains("must not appear"));
    // Reopening reads disk again; preserve view state while accepting new history.
    a_history.history.push(HistoryEntry::Messages {
        items: vec![InputItem::Message {
            role: MessageRole::Assistant,
            content: InputMessageContent::Text("external new answer".into()),
        }],
    });
    sessions.apply_event(&mut app, &mut editor, opened(a.clone(), a_history));
    assert_eq!(editor.text(), "unsent A draft");
    assert_eq!(editor.images().len(), 1);
    assert!(
        !editor.undo(),
        "session replacement resets undo history while keeping the cached draft"
    );
    assert_eq!(app.focus, Focus::Transcript);
    assert!(!app.follow);
    assert_eq!(
        app.anchor,
        Some(Anchor {
            record: 1,
            offset: 0
        })
    );
    assert!(matches!(
        app.records[1],
        Record::Reasoning { expanded: true, .. }
    ));
    assert!(format!("{:?}", app.records).contains("external new answer"));
    assert_eq!(app.session.as_ref().unwrap().id, a.id);
}

#[test]
fn reopening_a_session_preserves_manual_activity_expansion() {
    let (a, a_history) = task_snapshot("Task A");
    let (b, b_history) = task_snapshot("Task B");
    let mut sessions = Sessions::default();
    let mut app = App::new();
    let mut editor = Editor::new();
    sessions.apply_event(&mut app, &mut editor, opened(a.clone(), a_history.clone()));
    assert!(matches!(
        &app.records[1],
        Record::Activity {
            expanded: false,
            manual: false,
            ..
        }
    ));
    app.toggle_record(1);
    assert!(app.activity_expanded(1));
    sessions.apply_event(&mut app, &mut editor, opened(b, b_history));
    sessions.apply_event(&mut app, &mut editor, opened(a, a_history));
    assert!(matches!(
        &app.records[1],
        Record::Activity {
            expanded: true,
            manual: true,
            ..
        }
    ));
}

#[test]
fn picker_is_on_demand_and_busy_switching_preserves_the_draft() {
    let (metadata, replay) = snapshot("Compiler work");
    let mut sessions = Sessions::default();
    let mut app = App::new();
    let mut editor = Editor::new();
    sessions.apply_event(&mut app, &mut editor, opened(metadata.clone(), replay));
    editor.insert_str("unfinished draft");
    let (handled, request) = sessions.key(
        &mut app,
        key_with(KeyCode::Char('s'), KeyModifiers::CONTROL),
    );
    assert!(handled);
    assert!(matches!(request, Some(SessionCommand::List)));
    sessions.apply_event(&mut app, &mut editor, SessionEvent::Listed(vec![]));
    app.running = true;
    let (handled, request) = sessions.key(&mut app, key(KeyCode::Char('n')));
    assert!(handled);
    assert!(
        matches!(request, Some(SessionCommand::Create)),
        "U03 permits creating B while the current projection runs"
    );
    assert!(app.running);
    assert_eq!(editor.text(), "unfinished draft");
    sessions.pending = false;
    app.running = false;
    let (_, request) = sessions.key(&mut app, key(KeyCode::Char('n')));
    assert!(matches!(request, Some(SessionCommand::Create)));
    sessions.apply_event(
        &mut app,
        &mut editor,
        SessionEvent::Error {
            operation: "open",
            error: crate::client::RuntimeError::new("not_found", "cannot open target"),
        },
    );
    assert_eq!(app.session.as_ref().unwrap().id, metadata.id);
    assert_eq!(editor.text(), "unfinished draft");
    sessions.pending = false;
    sessions.key(&mut app, key(KeyCode::Esc));
    assert!(sessions.picker.is_none());
}

#[test]
fn session_title_is_visible_without_shortcuts_cluttering_the_status_line() {
    let (metadata, replay) = snapshot("Compiler work");
    let mut sessions = Sessions::default();
    let mut app = App::new();
    let mut editor = Editor::new();
    sessions.apply_event(&mut app, &mut editor, opened(metadata, replay));
    let mut terminal = Terminal::new(TestBackend::new(110, 25)).unwrap();
    terminal
        .draw(|frame| view::draw(frame, &mut app, &editor))
        .unwrap();
    let screen = buffer_text(terminal.backend().buffer());
    assert!(screen.contains("Compiler work"));
    assert!(screen.contains("test-model"));
    assert!(screen.contains("1M"));
    assert!(!screen.contains("Ctrl+S"));
    assert!(!app.session_button_area.is_empty());
    sessions.apply_event(&mut app, &mut editor, SessionEvent::Listed(vec![]));
    terminal
        .draw(|frame| {
            view::draw(frame, &mut app, &editor);
            sessions.draw(frame, app.theme);
        })
        .unwrap();
    let screen = buffer_text(terminal.backend().buffer());
    assert!(screen.contains("Sessions"));
    assert!(screen.contains("n new"));
}

fn picker_summary(id: &str, title: &str, model: &str, directory: &str) -> SessionSummary {
    let (mut metadata, _) = snapshot(title);
    metadata.id = id.to_owned();
    metadata.model = model.to_owned();
    metadata.working_directory = std::path::PathBuf::from(directory);
    SessionSummary {
        metadata,
        updated_at: 0,
        status: TaskStatus::Completed,
        in_use: false,
        unavailable: None,
    }
}

fn listed(items: Vec<SessionSummary>) -> (Sessions, App, Editor) {
    let mut sessions = Sessions::default();
    let mut app = App::new();
    let mut editor = Editor::new();
    sessions.apply_event(&mut app, &mut editor, SessionEvent::Listed(items));
    (sessions, app, editor)
}

fn search_and_open(
    sessions: &mut Sessions,
    app: &mut App,
    query: &str,
    downs: usize,
) -> Option<String> {
    sessions.key(app, key(KeyCode::Char('/')));
    for character in query.chars() {
        sessions.key(app, key(KeyCode::Char(character)));
    }
    for _ in 0..downs {
        sessions.key(app, key(KeyCode::Down));
    }
    match sessions.key(app, key(KeyCode::Enter)).1 {
        Some(SessionCommand::Open { session_id }) => Some(session_id),
        _ => None,
    }
}

fn rendered(sessions: &Sessions, app: &App, width: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, 18)).unwrap();
    terminal
        .draw(|frame| sessions.draw(frame, app.theme))
        .unwrap();
    buffer_text(terminal.backend().buffer())
}

#[test]
fn picker_search_matches_title_id_directory_and_model() {
    let compiler = picker_summary("id-one", "Compiler work", "model-alpha", "/work/alpha");
    let release = picker_summary("id-two", "Release notes", "model-beta", "/work/beta");
    let ops = picker_summary("id-three", "Deploy checklist", "model-gamma", "/work/gamma");
    let items = vec![compiler, release, ops];

    let (mut sessions, mut app, _) = listed(items.clone());
    assert_eq!(
        search_and_open(&mut sessions, &mut app, "COMPILER", 0),
        Some("id-one".to_owned())
    );

    let (mut sessions, mut app, _) = listed(items.clone());
    assert_eq!(
        search_and_open(&mut sessions, &mut app, "id-three", 0),
        Some("id-three".to_owned())
    );

    let (mut sessions, mut app, _) = listed(items.clone());
    assert_eq!(
        search_and_open(&mut sessions, &mut app, "work/beta", 0),
        Some("id-two".to_owned())
    );

    let (mut sessions, mut app, _) = listed(items.clone());
    assert_eq!(
        search_and_open(&mut sessions, &mut app, "model-gamma", 0),
        Some("id-three".to_owned())
    );

    // Two whitespace-separated terms must both match the same session.
    let (mut sessions, mut app, _) = listed(items.clone());
    assert_eq!(
        search_and_open(&mut sessions, &mut app, "compiler work", 0),
        Some("id-one".to_owned())
    );
    let (mut sessions, mut app, _) = listed(items);
    assert!(
        search_and_open(&mut sessions, &mut app, "compiler release", 0).is_none(),
        "terms do not combine across sessions"
    );
}

#[test]
fn picker_search_navigates_only_visible_results() {
    let alpha = picker_summary("id-a", "ticket alpha", "model", "/dir-a");
    let beta = picker_summary("id-b", "ticket beta", "model", "/dir-b");
    let gamma = picker_summary("id-c", "unrelated gamma", "model", "/dir-c");
    let items = vec![alpha, beta, gamma];

    let (mut sessions, mut app, _) = listed(items.clone());
    assert_eq!(
        search_and_open(&mut sessions, &mut app, "ticket", 1),
        Some("id-b".to_owned())
    );

    // Down past the last visible result stays on it.
    let (mut sessions, mut app, _) = listed(items.clone());
    assert_eq!(
        search_and_open(&mut sessions, &mut app, "ticket", 9),
        Some("id-b".to_owned())
    );

    let (mut sessions, mut app, _) = listed(items);
    assert_eq!(
        search_and_open(&mut sessions, &mut app, "gamma", 5),
        Some("id-c".to_owned())
    );
}

#[test]
fn picker_search_treats_n_and_r_as_text_and_esc_exits_then_closes() {
    let (mut sessions, mut app, _) = listed(vec![picker_summary("id-a", "alpha", "model", "/a")]);
    sessions.key(&mut app, key(KeyCode::Char('/')));
    // n and r edit the query; they never create or rename.
    for character in "nr".chars() {
        let (_, request) = sessions.key(&mut app, key(KeyCode::Char(character)));
        assert!(
            request.is_none() && !sessions.pending,
            "'{character}' is query text"
        );
    }
    let (_, request) = sessions.key(&mut app, key(KeyCode::Enter));
    assert!(
        request.is_none() && !sessions.pending,
        "no match does not confirm"
    );
    // First Esc leaves search and keeps the picker open.
    sessions.key(&mut app, key(KeyCode::Esc));
    assert!(sessions.picker.is_some());
    let (_, request) = sessions.key(&mut app, key(KeyCode::Enter));
    assert!(matches!(request, Some(SessionCommand::Open { .. })));
    // Second Esc closes the picker.
    sessions.pending = false;
    sessions.key(&mut app, key(KeyCode::Esc));
    assert!(sessions.picker.is_none());
}

#[test]
fn picker_search_ctrl_c_exits_search_before_closing() {
    let (mut sessions, mut app, _) = listed(vec![picker_summary("id-a", "alpha", "model", "/a")]);
    sessions.key(&mut app, key(KeyCode::Char('/')));
    sessions.key(&mut app, key(KeyCode::Char('x')));
    sessions.key(
        &mut app,
        key_with(KeyCode::Char('c'), KeyModifiers::CONTROL),
    );
    assert!(sessions.picker.is_some());
    sessions.key(
        &mut app,
        key_with(KeyCode::Char('c'), KeyModifiers::CONTROL),
    );
    assert!(sessions.picker.is_none());
}

#[test]
fn picker_search_no_match_shows_empty_state_and_enter_does_nothing() {
    let (mut sessions, mut app, _) = listed(vec![picker_summary("id-a", "alpha", "model", "/a")]);
    sessions.key(&mut app, key(KeyCode::Char('/')));
    for character in "zzz-no-such".chars() {
        sessions.key(&mut app, key(KeyCode::Char(character)));
    }
    let (_, request) = sessions.key(&mut app, key(KeyCode::Enter));
    assert!(request.is_none() && !sessions.pending);
    // 'r' stays literal in search, so Enter cannot start a rename.
    sessions.key(&mut app, key(KeyCode::Char('r')));
    let (_, request) = sessions.key(&mut app, key(KeyCode::Enter));
    assert!(request.is_none() && !sessions.pending);
    assert!(sessions.picker.is_some());
    let screen = rendered(&sessions, &app, 60);
    assert!(screen.contains("Search:"), "{screen}");
    assert!(screen.contains("No sessions match"), "{screen}");
}

#[test]
fn picker_search_backspace_is_grapheme_safe() {
    let (mut sessions, mut app, _) = listed(vec![picker_summary("id-a", "alpha", "model", "/a")]);
    sessions.key(&mut app, key(KeyCode::Char('/')));
    for character in "cafe\u{301}".chars() {
        sessions.key(&mut app, key(KeyCode::Char(character)));
    }
    let screen = rendered(&sessions, &app, 60);
    assert!(screen.contains("Search: cafe\u{301}"), "{screen}");
    sessions.key(&mut app, key(KeyCode::Backspace));
    let screen = rendered(&sessions, &app, 60);
    assert!(screen.contains("Search: caf"), "{screen}");
    assert!(
        !screen.contains("cafe"),
        "the combining grapheme is gone: {screen}"
    );
}

#[test]
fn picker_search_recomputes_and_preserves_selection_on_title_change() {
    let alpha = picker_summary("id-a", "alpha one", "model", "/a");
    let beta = picker_summary("id-b", "alpha two", "model", "/b");
    let (mut sessions, mut app, mut editor) = listed(vec![alpha, beta.clone()]);
    sessions.key(&mut app, key(KeyCode::Char('/')));
    for character in "alpha".chars() {
        sessions.key(&mut app, key(KeyCode::Char(character)));
    }
    sessions.key(&mut app, key(KeyCode::Down));
    // A title update that keeps the second result matching preserves it.
    let mut updated = beta.metadata.clone();
    updated.title = "alpha two revised".into();
    sessions.apply_event(&mut app, &mut editor, SessionEvent::TitleUpdated(updated));
    let (_, request) = sessions.key(&mut app, key(KeyCode::Enter));
    assert!(matches!(request, Some(SessionCommand::Open { session_id }) if session_id == "id-b"));
    // A rename that removes it from the result set falls back to the first.
    sessions.pending = false;
    let mut renamed = beta.metadata.clone();
    renamed.title = "beta two".into();
    sessions.apply_event(&mut app, &mut editor, SessionEvent::Renamed(renamed));
    let (_, request) = sessions.key(&mut app, key(KeyCode::Enter));
    assert!(matches!(request, Some(SessionCommand::Open { session_id }) if session_id == "id-a"));
}

#[test]
fn picker_rename_target_stays_stable_across_title_updates() {
    let alpha = picker_summary("id-a", "alpha one", "model", "/a");
    let beta = picker_summary("id-b", "alpha two", "model", "/b");
    let (mut sessions, mut app, mut editor) = listed(vec![alpha.clone(), beta]);
    sessions.key(&mut app, key(KeyCode::Char('/')));
    for character in "alpha".chars() {
        sessions.key(&mut app, key(KeyCode::Char(character)));
    }
    sessions.key(&mut app, key(KeyCode::Down));
    sessions.key(&mut app, key(KeyCode::Esc)); // leave search, selection preserved
    sessions.key(&mut app, key(KeyCode::Char('r'))); // rename id-b
    // A late automatic title for a different session must not redirect it.
    let mut updated = alpha.metadata.clone();
    updated.title = "alpha one revised".into();
    sessions.apply_event(&mut app, &mut editor, SessionEvent::TitleUpdated(updated));
    let (_, request) = sessions.key(&mut app, key(KeyCode::Enter));
    assert!(
        matches!(request, Some(SessionCommand::Rename { session_id, .. }) if session_id == "id-b")
    );
}

#[test]
fn picker_search_preserves_draft_images_and_reading_position() {
    let (mut sessions, mut app, mut editor) =
        listed(vec![picker_summary("id-a", "alpha", "model", "/a")]);
    editor.set_draft(crate::input::UserInput {
        context: Vec::new(),
        text: "keep this draft".into(),
        images: vec![ImageAttachment::new("a.png", "data:image/png;base64,AA")],
    });
    app.follow = false;
    app.anchor = Some(Anchor {
        record: 0,
        offset: 0,
    });
    sessions.key(&mut app, key(KeyCode::Char('/')));
    for character in "alp".chars() {
        sessions.key(&mut app, key(KeyCode::Char(character)));
    }
    sessions.key(&mut app, key(KeyCode::Esc));
    sessions.key(&mut app, key(KeyCode::Esc));
    assert!(sessions.picker.is_none());
    assert_eq!(editor.text(), "keep this draft");
    assert_eq!(editor.images().len(), 1);
    assert!(!app.follow);
    assert_eq!(
        app.anchor,
        Some(Anchor {
            record: 0,
            offset: 0
        })
    );
}

#[test]
fn picker_search_renders_field_at_narrow_and_wide_widths() {
    let alpha = picker_summary("id-a", "alpha project", "model-x", "/long/dir/alpha");
    let beta = picker_summary("id-b", "beta project", "model-y", "/long/dir/beta");
    let (mut sessions, mut app, _) = listed(vec![alpha, beta]);

    for width in [40u16, 110] {
        let screen = rendered(&sessions, &app, width);
        assert!(screen.contains("Sessions"), "width {width}: {screen}");
        assert!(screen.contains("/ search"), "width {width}: {screen}");
    }

    sessions.key(&mut app, key(KeyCode::Char('/')));
    for character in "alpha".chars() {
        sessions.key(&mut app, key(KeyCode::Char(character)));
    }
    for width in [40u16, 110] {
        let screen = rendered(&sessions, &app, width);
        assert!(screen.contains("Search:"), "width {width}: {screen}");
        assert!(screen.contains("alpha project"), "width {width}: {screen}");
        assert!(!screen.contains("beta project"), "width {width}: {screen}");
    }
}

#[test]
fn picker_search_keeps_the_query_tail_and_cursor_visible_when_narrow() {
    let (mut sessions, mut app, _) = listed(vec![picker_summary("id-a", "alpha", "model", "/a")]);
    sessions.key(&mut app, key(KeyCode::Char('/')));
    let prefix = "汉".repeat(10);
    let suffix = "zq7x9 rocket e\u{301}";
    for character in format!("{prefix}{suffix}").chars() {
        sessions.key(&mut app, key(KeyCode::Char(character)));
    }
    let screen = rendered(&sessions, &app, 40);
    assert!(screen.contains("Search:"), "{screen}");
    assert!(
        screen.contains("zq7x9 rocket e\u{301}▌"),
        "the entire fitting suffix and caret are visible: {screen}"
    );
    // Editing keeps working at the visible tail.
    sessions.key(&mut app, key(KeyCode::Backspace));
    let screen = rendered(&sessions, &app, 40);
    assert!(
        screen.contains("zq7x9 rocket ▌"),
        "suffix and caret stay visible after grapheme Backspace: {screen}"
    );
    assert!(
        !screen.contains("e\u{301}"),
        "backspace removes the combining grapheme: {screen}"
    );
}

#[test]
fn opening_a_session_closes_an_active_transcript_search() {
    let (a, a_history) = snapshot("Session A");
    let (b, b_history) = snapshot("Session B");
    let mut sessions = Sessions::default();
    let mut app = App::new();
    let mut editor = Editor::new();
    sessions.apply_event(&mut app, &mut editor, opened(a, a_history));
    app.last_width = 40;
    app.last_height = 6;
    crate::frontend::tui::transcript_search::open(&mut app);
    assert!(crate::frontend::tui::transcript_search::active(&app));
    sessions.apply_event(&mut app, &mut editor, opened(b, b_history));
    assert!(
        !crate::frontend::tui::transcript_search::active(&app),
        "session replacement never caches an active search"
    );
}

#[test]
fn session_switch_never_caches_a_path_completion_popup() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("saki28-session-{unique}"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir(dir.join("alpha-dir")).unwrap();
    std::fs::write(dir.join("alpha-file.txt"), "x").unwrap();

    let (a, a_history) = snapshot("Session A");
    let (b, b_history) = snapshot("Session B");
    let mut sessions = Sessions::default();
    let mut app = App::new();
    let mut editor = Editor::new();
    sessions.apply_event(&mut app, &mut editor, opened(a.clone(), a_history));
    app.working_directory = Some(dir.clone());
    app.last_width = 80;
    app.last_height = 20;
    editor.set_draft("open a");
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert!(crate::frontend::tui::path_completion::active(&app));

    sessions.apply_event(&mut app, &mut editor, opened(b, b_history));
    assert!(
        !crate::frontend::tui::path_completion::active(&app),
        "a different cwd must not inherit the cached popup"
    );
    sessions.apply_event(&mut app, &mut editor, opened(a, snapshot("Session A").1));
    assert!(
        !crate::frontend::tui::path_completion::active(&app),
        "reopening the cached session must not resurrect the popup"
    );
    std::fs::remove_dir_all(dir).ok();
}

fn association(id: &str, task: &str) -> crate::client::Association {
    crate::client::Association {
        session_id: id.to_owned(),
        task_id: task.to_owned(),
        message_id: format!("{task}-message"),
    }
}

fn opened_stream(
    metadata: SessionMetadata,
    replay: History,
    stream: crate::client::Association,
) -> SessionEvent {
    let mut state = idle_state(&metadata.id);
    state.state = "running".into();
    state.active_task = Some(crate::client::types::ActiveTaskState {
        task_id: stream.task_id.clone(),
        message_id: stream.message_id.clone(),
        cancel_requested: false,
    });
    SessionEvent::Opened {
        metadata,
        replay,
        context: AgentEvent::ContextUpdated {
            model: "test-model".into(),
            context_tokens: Some(1000),
            context_window: Some(1_000_000),
            input_limit: Some(991_808),
        },
        state,
        stream: Some(stream),
    }
}

fn finished(status: &'static str) -> crate::client::TaskFinished {
    crate::client::TaskFinished {
        status: status.into(),
        error: None,
        persistence: None,
        history_required: false,
    }
}

#[test]
fn cached_live_a_survives_view_b_and_return_before_finished() {
    let (a, ah) = snapshot("Session A");
    let (b, bh) = snapshot("Session B");
    let mut sessions = Sessions::default();
    let mut app = App::new();
    let mut editor = Editor::new();
    sessions.apply_event(&mut app, &mut editor, opened(a.clone(), ah.clone()));
    let live = association(&a.id, "task-a");
    sessions.accept_send(
        &mut app,
        &mut editor,
        live.clone(),
        &crate::input::UserInput::text("A is running"),
    );
    assert!(app.running);
    let delta = |item: &str, value: &str| SessionEvent::Agent {
        session_id: a.id.clone(),
        event: AgentEvent::ResponseTextDelta {
            item_id: item.into(),
            delta: value.into(),
        },
    };
    sessions.apply_event_with_association(
        &mut app,
        &mut editor,
        delta("same-item", "live-"),
        Some(live.clone()),
    );
    sessions.apply_event(&mut app, &mut editor, opened(b.clone(), bh));
    assert_eq!(app.session.as_ref().unwrap().id, b.id);
    assert!(!app.running, "B is idle while A runs");
    sessions.apply_event_with_association(
        &mut app,
        &mut editor,
        delta("same-item", "background"),
        Some(live.clone()),
    );
    assert!(
        !format!("{:?}", app.records).contains("background"),
        "B must not receive A's event"
    );
    sessions.apply_event(
        &mut app,
        &mut editor,
        opened_stream(a.clone(), ah, live.clone()),
    );
    let text = format!("{:?}", app.records);
    assert!(text.contains("live-"), "A's live stream is preserved");
    assert!(
        text.contains("background"),
        "offscreen A events were projected"
    );
    assert!(
        !text.contains("live-background") || text.matches("background").count() == 1,
        "late Opened does not duplicate streamed output"
    );
    sessions.apply_event_with_association(
        &mut app,
        &mut editor,
        SessionEvent::Finished(finished("completed")),
        Some(live),
    );
    assert!(!app.running);
    assert!(sessions.current_association().is_none());
}

#[test]
fn reused_item_and_call_ids_do_not_cross_accepted_tasks() {
    let (metadata, replay) = snapshot("Tasks");
    let mut sessions = Sessions::default();
    let mut app = App::new();
    let mut editor = Editor::new();
    sessions.apply_event(&mut app, &mut editor, opened(metadata.clone(), replay));
    let first = association(&metadata.id, "task-1");
    sessions.accept_send(
        &mut app,
        &mut editor,
        first.clone(),
        &crate::input::UserInput::text("first"),
    );
    sessions.apply_event_with_association(
        &mut app,
        &mut editor,
        SessionEvent::Agent {
            session_id: metadata.id.clone(),
            event: AgentEvent::ResponseTextDelta {
                item_id: "reused".into(),
                delta: "first-delta".into(),
            },
        },
        Some(first.clone()),
    );
    sessions.apply_event_with_association(
        &mut app,
        &mut editor,
        SessionEvent::Agent {
            session_id: metadata.id.clone(),
            event: AgentEvent::ToolStarted {
                call_id: "reused-call".into(),
                name: "bash".into(),
                arguments: "{}".into(),
            },
        },
        Some(first),
    );
    let second = association(&metadata.id, "task-2");
    sessions.accept_send(
        &mut app,
        &mut editor,
        second.clone(),
        &crate::input::UserInput::text("second"),
    );
    sessions.apply_event_with_association(
        &mut app,
        &mut editor,
        SessionEvent::Agent {
            session_id: metadata.id.clone(),
            event: AgentEvent::ResponseTextDelta {
                item_id: "reused".into(),
                delta: "second-delta".into(),
            },
        },
        Some(second.clone()),
    );
    sessions.apply_event_with_association(
        &mut app,
        &mut editor,
        SessionEvent::Agent {
            session_id: metadata.id.clone(),
            event: AgentEvent::ToolFinished {
                call_id: "reused-call".into(),
                name: "bash".into(),
                output: serde_json::json!({"exit_code": 0}),
            },
        },
        Some(second),
    );
    let assistants: Vec<_> = app
        .records
        .iter()
        .filter_map(|record| match record {
            Record::Assistant { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(assistants.contains(&"first-delta"));
    assert!(assistants.contains(&"second-delta"));
    assert!(
        !assistants
            .iter()
            .any(|text| text.contains("first-deltasecond-delta")),
        "same item id from a new task must not append to the old record"
    );
    let tools: Vec<_> = app
        .records
        .iter()
        .filter_map(|record| match record {
            Record::Tool { output, .. } => Some(output.is_some()),
            _ => None,
        })
        .collect();
    assert_eq!(tools, vec![false, true], "old unfinished call stays old");
}

#[test]
fn stale_finished_cannot_finish_a_newer_task_and_history_repair_does_not_select() {
    let (metadata, replay) = snapshot("Stale");
    let (other, other_replay) = snapshot("Other");
    let mut sessions = Sessions::default();
    let mut app = App::new();
    let mut editor = Editor::new();
    sessions.apply_event(
        &mut app,
        &mut editor,
        opened(metadata.clone(), replay.clone()),
    );
    let first = association(&metadata.id, "task-1");
    sessions.accept_send(
        &mut app,
        &mut editor,
        first.clone(),
        &crate::input::UserInput::text("first"),
    );
    sessions.apply_event_with_association(
        &mut app,
        &mut editor,
        SessionEvent::Finished(finished("completed")),
        Some(first.clone()),
    );
    let second = association(&metadata.id, "task-2");
    sessions.accept_send(
        &mut app,
        &mut editor,
        second.clone(),
        &crate::input::UserInput::text("second"),
    );
    sessions.apply_event_with_association(
        &mut app,
        &mut editor,
        SessionEvent::Finished(finished("completed")),
        Some(first),
    );
    assert!(app.running, "stale Finished cannot finalize the newer task");
    assert_eq!(
        sessions
            .current_association()
            .as_ref()
            .map(|a| a.task_id.as_str()),
        Some("task-2")
    );

    sessions.apply_event(&mut app, &mut editor, opened(other.clone(), other_replay));
    assert_eq!(app.session.as_ref().unwrap().id, other.id);
    sessions.apply_event_with_association(
        &mut app,
        &mut editor,
        SessionEvent::HistoryRestored {
            metadata: metadata.clone(),
            replay,
            context: AgentEvent::ContextUpdated {
                model: "test-model".into(),
                context_tokens: Some(1),
                context_window: Some(2),
                input_limit: Some(1),
            },
            state: idle_state(&metadata.id),
        },
        Some(second.clone()),
    );
    assert_eq!(
        app.session.as_ref().unwrap().id,
        other.id,
        "HistoryRestored must not select the repaired session"
    );
    sessions.apply_event_with_association(
        &mut app,
        &mut editor,
        SessionEvent::Finished(crate::client::TaskFinished {
            status: "failed".into(),
            error: Some(crate::client::RuntimeError::new(
                "model_error",
                "late failure",
            )),
            persistence: Some("unconfirmed".into()),
            history_required: true,
        }),
        Some(second),
    );
    assert!(
        !app.running,
        "offscreen repair/finish must not make idle B run"
    );
    assert_eq!(app.session.as_ref().unwrap().id, other.id);
    assert!(sessions.current_association().is_none());
}

#[test]
fn cancel_acknowledgment_never_finishes_the_task() {
    let (metadata, replay) = snapshot("Cancel");
    let mut sessions = Sessions::default();
    let mut app = App::new();
    let mut editor = Editor::new();
    sessions.apply_event(&mut app, &mut editor, opened(metadata.clone(), replay));
    let live = association(&metadata.id, "task");
    sessions.accept_send(
        &mut app,
        &mut editor,
        live.clone(),
        &crate::input::UserInput::text("run"),
    );
    sessions.apply_event_with_association(
        &mut app,
        &mut editor,
        SessionEvent::Cancelled {
            status: "requested".into(),
        },
        Some(live.clone()),
    );
    assert!(app.running, "cancel ack is not a terminal");
    sessions.apply_event_with_association(
        &mut app,
        &mut editor,
        SessionEvent::Cancelled {
            status: "not_running".into(),
        },
        Some(live.clone()),
    );
    assert!(app.running, "not_running ack is not a Finished event");
    assert!(sessions.current_association().is_some());
}

#[test]
fn full_message_association_guards_agent_and_finished() {
    let (metadata, replay) = snapshot("Assoc");
    let mut sessions = Sessions::default();
    let mut app = App::new();
    let mut editor = Editor::new();
    sessions.apply_event(&mut app, &mut editor, opened(metadata.clone(), replay));
    let live = association(&metadata.id, "task");
    sessions.accept_send(
        &mut app,
        &mut editor,
        live.clone(),
        &crate::input::UserInput::text("run"),
    );
    let mut stranger = live.clone();
    stranger.message_id = "different-message".into();
    sessions.apply_event_with_association(
        &mut app,
        &mut editor,
        SessionEvent::Agent {
            session_id: metadata.id.clone(),
            event: AgentEvent::ResponseTextDelta {
                item_id: "x".into(),
                delta: "must not apply".into(),
            },
        },
        Some(stranger.clone()),
    );
    assert!(!format!("{:?}", app.records).contains("must not apply"));
    sessions.apply_event_with_association(
        &mut app,
        &mut editor,
        SessionEvent::Finished(finished("completed")),
        Some(stranger),
    );
    assert!(app.running, "mismatched message_id cannot finish the task");
    assert!(sessions.current_association().is_some());
}

#[test]
fn duplicate_finished_and_canonical_repair_keep_error_and_persistence_visible() {
    let (metadata, replay) = snapshot("Canonical");
    let mut sessions = Sessions::default();
    let mut app = App::new();
    let mut editor = Editor::new();
    sessions.apply_event(
        &mut app,
        &mut editor,
        opened(metadata.clone(), replay.clone()),
    );
    let live = association(&metadata.id, "task");
    sessions.accept_send(
        &mut app,
        &mut editor,
        live.clone(),
        &crate::input::UserInput::text("run"),
    );
    sessions.apply_event_with_association(
        &mut app,
        &mut editor,
        SessionEvent::HistoryRestored {
            metadata: metadata.clone(),
            replay,
            context: AgentEvent::ContextUpdated {
                model: "test-model".into(),
                context_tokens: Some(1),
                context_window: Some(2),
                input_limit: Some(1),
            },
            state: idle_state(&metadata.id),
        },
        Some(live.clone()),
    );
    let failed = crate::client::TaskFinished {
        status: "failed".into(),
        error: Some(crate::client::RuntimeError::new(
            "model_error",
            "canonical failure",
        )),
        persistence: Some("unconfirmed".into()),
        history_required: true,
    };
    sessions.apply_event_with_association(
        &mut app,
        &mut editor,
        SessionEvent::Finished(failed.clone()),
        Some(live.clone()),
    );
    assert!(!app.running);
    let feedback = app
        .feedback
        .as_ref()
        .map(|(message, _)| message.clone())
        .unwrap_or_default();
    assert!(feedback.contains("canonical failure"), "{feedback}");
    assert!(feedback.contains("unconfirmed"), "{feedback}");
    let records = app.records.len();
    sessions.apply_event_with_association(
        &mut app,
        &mut editor,
        SessionEvent::Finished(failed),
        Some(live),
    );
    assert_eq!(
        app.records.len(),
        records,
        "duplicate Finished has no display effect"
    );
}

#[test]
fn full_request_queue_rejects_without_tracking() {
    use crate::frontend::session_requests::{PendingRequest, SessionRequests};
    let (sender, _receiver) = tokio::sync::mpsc::channel(1);
    let mut requests = SessionRequests::new(sender);
    let first = requests
        .try_send(SessionCommand::List, PendingRequest::Control)
        .expect("first request fits");
    assert_eq!(first, 1);
    let rejected = requests.try_send(
        SessionCommand::Send {
            session_id: "s".into(),
            message: crate::input::UserInput::text("draft"),
        },
        PendingRequest::Send {
            session_id: "s".into(),
            message: crate::input::UserInput::text("draft"),
        },
    );
    assert!(
        rejected.is_err(),
        "full mailbox must reject without blocking"
    );
    assert!(matches!(
        requests.take(first),
        Some(PendingRequest::Control)
    ));
    assert!(
        requests.take(2).is_none(),
        "rejected request is not tracked"
    );
}

#[test]
fn same_current_opened_with_stream_preserves_live_app_editor_and_theme() {
    let (metadata, replay) = snapshot("Same");
    let mut sessions = Sessions::default();
    let mut app = App::new();
    let mut editor = Editor::new();
    sessions.apply_event(
        &mut app,
        &mut editor,
        opened(metadata.clone(), replay.clone()),
    );
    app.theme = crate::frontend::tui::theme::Theme::light();
    let live = association(&metadata.id, "task");
    sessions.accept_send(
        &mut app,
        &mut editor,
        live.clone(),
        &crate::input::UserInput::text("run"),
    );
    sessions.apply_event_with_association(
        &mut app,
        &mut editor,
        SessionEvent::Agent {
            session_id: metadata.id.clone(),
            event: AgentEvent::ResponseTextDelta {
                item_id: "m".into(),
                delta: "live".into(),
            },
        },
        Some(live.clone()),
    );
    editor.insert_str("draft stays");
    let cursor = editor.cursor();
    let before = format!("{:?}", app.records);
    sessions.apply_event(
        &mut app,
        &mut editor,
        opened_stream(metadata.clone(), replay, live),
    );
    assert_eq!(app.session.as_ref().unwrap().id, metadata.id);
    assert_eq!(
        app.theme,
        crate::frontend::tui::theme::Theme::light(),
        "same-current reload keeps the live palette"
    );
    assert_eq!(editor.text(), "draft stays");
    assert_eq!(editor.cursor(), cursor);
    assert_eq!(
        format!("{:?}", app.records),
        before,
        "live records not rebuilt"
    );
    assert!(app.running);
}
