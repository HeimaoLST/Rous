use super::sessions::{opened, snapshot};
use super::*;
use crate::{
    command::Command,
    frontend::tui::{commands::Commands, sessions::Sessions},
    session::{SessionCommand, SessionEvent},
};

fn session_result(result: Option<(Command, SessionCommand)>) -> Option<SessionCommand> {
    result.map(|(_, request)| request)
}

#[test]
fn local_input_never_submits_a_user_message_and_errors_keep_images_and_text() {
    let mut app = App::new();
    let mut editor = Editor::new();
    editor.set_draft("/resuem");
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Enter));
    assert!(outcome.send.is_none() && outcome.command.is_none());
    assert_eq!(editor.text(), "/resuem");
    assert!(!app.running);
    assert!(app.records.is_empty());
    editor.set_draft(crate::input::UserInput {
        context: Vec::new(),
        text: "/new".into(),
        images: vec![crate::input::ImageAttachment::new(
            "a.png",
            "data:image/png;base64,AA",
        )],
    });
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Enter));
    assert!(outcome.send.is_none() && outcome.command.is_none());
    assert_eq!(editor.images().len(), 1);
    assert_eq!(editor.text(), "/new");
    assert!(app.feedback.as_ref().unwrap().0.contains("images"));
    editor.set_draft("/help");
    app.running = true;
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Enter));
    assert_eq!(outcome.command, Some(Command::Help));
    assert!(outcome.send.is_none());
    let mut commands = Commands::default();
    let mut sessions = Sessions::default();
    assert!(
        commands
            .dispatch(
                outcome.command.unwrap(),
                &mut app,
                &mut editor,
                &mut sessions
            )
            .is_none()
    );
    assert!(app.help_open && app.running);
    assert!(editor.is_empty());
    assert!(app.records.is_empty());
}

#[test]
fn failed_switch_keeps_the_command_but_success_does_not_cache_it_as_a_draft() {
    let (a, ah) = snapshot("A");
    let (b, bh) = snapshot("B");
    let mut app = App::new();
    let mut editor = Editor::new();
    let mut sessions = Sessions::default();
    let mut commands = Commands::default();
    sessions.apply_event(&mut app, &mut editor, opened(a.clone(), ah.clone()));
    editor.set_draft("/new");
    assert!(matches!(
        session_result(commands.dispatch(Command::New, &mut app, &mut editor, &mut sessions)),
        Some(SessionCommand::Create)
    ));
    let error = SessionEvent::Error {
        operation: "create",
        error: crate::client::RuntimeError::new("busy", "cannot create session"),
    };
    sessions.apply_event(&mut app, &mut editor, error);
    assert_eq!(editor.text(), "/new");
    assert_eq!(app.session.as_ref().unwrap().id, a.id);
    assert!(
        sessions.pending,
        "an async Error does not acknowledge the command"
    );
    sessions.pending = false;
    commands.dispatch(Command::New, &mut app, &mut editor, &mut sessions);
    editor.reset();
    let event = opened(b, bh);
    sessions.apply_event(&mut app, &mut editor, event);
    assert!(
        editor.is_empty(),
        "switching uses B's editor, not A's draft"
    );
    sessions.apply_event(&mut app, &mut editor, opened(a, ah));
    assert!(
        editor.is_empty(),
        "executed /new must not reappear when returning to A"
    );
}

#[test]
fn busy_controls_and_automatic_title_updates_do_not_finish_the_active_job_or_clear_commands() {
    let (mut metadata, replay) = snapshot("A");
    let mut app = App::new();
    let mut editor = Editor::new();
    let mut sessions = Sessions::default();
    let mut commands = Commands::default();
    sessions.apply_event(&mut app, &mut editor, opened(metadata.clone(), replay));
    app.running = true;
    editor.set_draft("/new");
    assert!(
        session_result(commands.dispatch(Command::New, &mut app, &mut editor, &mut sessions))
            .is_some(),
        "U03 allows creating/viewing B while the current projection runs"
    );
    assert_eq!(editor.text(), "/new");
    assert!(app.running);
    assert!(
        sessions.pending,
        "command tracks the host request until its reply"
    );
    editor.set_draft("/rename active");
    assert!(matches!(
        session_result(commands.dispatch(
            Command::Rename {
                title: "active".into()
            },
            &mut app,
            &mut editor,
            &mut sessions
        )),
        Some(SessionCommand::Rename { .. })
    ));
    metadata.title = "active".into();
    let automatic = SessionEvent::TitleUpdated(metadata.clone());
    sessions.apply_event(&mut app, &mut editor, automatic);
    assert!(sessions.pending && app.running);
    assert_eq!(editor.text(), "/rename active");
    let progress = SessionEvent::Agent {
        session_id: metadata.id.clone(),
        event: AgentEvent::ResponseTextDelta {
            item_id: "m".into(),
            delta: "working".into(),
        },
    };
    sessions.apply_event(&mut app, &mut editor, progress);
    assert!(sessions.pending && app.running);
    assert_eq!(editor.text(), "/rename active");
    let manual = SessionEvent::Renamed(metadata);
    sessions.apply_event(&mut app, &mut editor, manual);
    assert!(
        sessions.pending,
        "metadata updates do not acknowledge commands"
    );
    assert_eq!(editor.text(), "/rename active");
    assert!(app.running);
}

#[test]
fn completion_navigation_and_dismissal_preserve_input_and_tab_focus_behavior() {
    let mut app = App::new();
    let mut editor = Editor::new();
    let mut commands = Commands::default();
    editor.set_draft("/");
    commands.sync(&app, &editor);
    assert!(commands.key(&mut editor, key(KeyCode::Down)));
    assert!(commands.key(&mut editor, key(KeyCode::Down)));
    assert!(commands.key(&mut editor, key(KeyCode::Tab)));
    assert_eq!(editor.text(), "/resume ");
    assert_eq!(app.focus, Focus::Input);
    editor.set_draft("/re");
    commands.sync(&app, &editor);
    assert!(commands.key(&mut editor, key(KeyCode::Esc)));
    assert_eq!(editor.text(), "/re");
    commands.sync(&app, &editor);
    assert!(!commands.key(&mut editor, key(KeyCode::Tab)));
    handle_key(&mut app, &mut editor, key(KeyCode::Tab));
    assert_eq!(app.focus, Focus::Transcript);
    app.focus = Focus::Input;
    editor.set_draft("ordinary input");
    commands.sync(&app, &editor);
    assert!(!commands.key(&mut editor, key(KeyCode::Up)));
    assert!(app.records.is_empty());
}

#[test]
fn completion_is_drawn_above_input_and_mouse_click_completes_without_sending() {
    let mut app = App::new();
    let mut editor = Editor::new();
    let mut commands = Commands::default();
    editor.set_draft("/re");
    commands.sync(&app, &editor);
    let mut terminal = Terminal::new(TestBackend::new(90, 24)).unwrap();
    terminal
        .draw(|frame| {
            view::draw(frame, &mut app, &editor);
            commands.draw(frame, &app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let screen = buffer_text(buffer);
    assert!(screen.contains("Commands"));
    assert!(screen.contains("/resume") && screen.contains("/rename"));
    let row = screen
        .lines()
        .position(|line| line.contains("/rename"))
        .unwrap();
    assert!(row < (app.input_area.y as usize));
    assert!(commands.mouse(
        &mut editor,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 3,
            row: row as u16,
            modifiers: KeyModifiers::NONE
        }
    ));
    assert_eq!(editor.text(), "/rename ");
    assert!(!app.running);
    assert!(app.records.is_empty());
    commands.sync(&app, &editor);
    terminal
        .draw(|frame| {
            view::draw(frame, &mut app, &editor);
            commands.draw(frame, &app);
        })
        .unwrap();
    assert!(!buffer_text(terminal.backend().buffer()).contains("Commands"));
}

#[test]
fn multiline_paste_and_escaped_slashes_use_the_existing_message_submission_path() {
    let mut app = App::new();
    let mut editor = Editor::new();
    handle_paste_text(&mut app, &mut editor, "/new\nexplain this text");
    assert!(!app.running && app.records.is_empty());
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Enter));
    assert!(outcome.command.is_none());
    assert_eq!(outcome.send.unwrap().text, "/new\nexplain this text");
    editor.set_draft(crate::input::UserInput {
        context: Vec::new(),
        text: "//help".into(),
        images: vec![crate::input::ImageAttachment::new(
            "a.png",
            "data:image/png;base64,AA",
        )],
    });
    let outcome = handle_key(&mut app, &mut editor, key(KeyCode::Enter));
    let message = outcome.send.unwrap();
    assert_eq!(message.text, "/help");
    assert_eq!(message.images.len(), 1);
    assert_eq!(editor.text(), "//help", "draft remains until Accepted");
}
