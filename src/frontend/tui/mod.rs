pub mod code_highlight;
pub mod commands;
pub mod editor;
pub mod external_editor;
pub mod help;
pub mod history;
pub mod log;
pub mod markdown;
pub mod markdown_layout;
pub mod path_completion;
pub mod selection;
pub mod sessions;
pub mod state;
pub mod terminal;
pub mod terminal_input;
pub mod textwrap;
pub mod theme;
pub mod transcript_search;
pub mod view;

use std::path::PathBuf;

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use tokio::sync::mpsc::Sender;

use crate::command::{self, Command, ParsedInput};
use crate::frontend::session_requests::{PendingRequest, RequestSendError, SessionRequests};
use crate::input::{self as message, ImageAttachment, UserInput};
use crate::session::{SessionCommand, SessionEvent, SessionEventReceiver, SessionRequest};
use editor::Editor;
use state::{App, Focus};
use terminal_input::Input;

/// Terminal front end.
///
/// It consumes session snapshots and tagged Agent events through the channels
/// the worker provides. It never holds the agent, conversation or tools.
pub async fn run(
    input: Sender<SessionRequest>,
    mut events: SessionEventReceiver,
) -> anyhow::Result<()> {
    // Theme selection happens once; App.theme is the only rendered state.
    // Manual settings never send queries or subscribe to scheme notifications.
    let setting = theme::ThemeSetting::from_env();
    let mut app = App::new();
    app.theme = setting.initial_theme();
    let mut session = terminal::TerminalSession::enter(setting)?;
    let mut sessions = sessions::Sessions::default();
    sessions.pending = true;
    let mut editor = Editor::new();
    let mut commands = commands::Commands::default();
    let mut requests = SessionRequests::new(input);
    let mut external_edit: Option<ExternalEdit> = None;
    let mut events_closed = false;

    loop {
        if external_edit.is_none() {
            commands.sync(&app, &editor);
            session.draw(|frame| {
                view::draw(frame, &mut app, &editor);
                if sessions.picker.is_none() && !sessions.pending {
                    if !app.history.searching() && !transcript_search::active(&app) {
                        commands.draw(frame, &app);
                        path_completion::draw(frame, &app);
                    }
                    history::draw(frame, &app);
                }
                sessions.draw(frame, app.theme);
            })?;
        }
        if events_closed && external_edit.is_none() {
            break;
        }
        if app.should_quit() && external_edit.is_none() {
            break;
        }
        let feedback_deadline = app
            .feedback
            .as_ref()
            .map(|(_, until)| tokio::time::Instant::from_std(*until));
        let scroll_deadline = app
            .text_selection
            .edge_deadline()
            .map(tokio::time::Instant::from_std);
        tokio::select! {
            result = async {
                match external_edit.as_mut() {
                    Some((receiver, _, _)) => receiver.await,
                    None => std::future::pending().await,
                }
            }, if external_edit.is_some() => {
                let (_, guard, target) = external_edit.take().unwrap();
                let resumed = session.resume_external();
                drop(guard);
                resumed?;
                match result {
                    Ok(Ok(text)) => {
                        if app
                            .session
                            .as_ref()
                            .is_some_and(|active| active.id == target)
                        {
                            editor.replace_text(&text);
                        } else {
                            sessions.replace_cached_text(&target, &text);
                        }
                    }
                    Ok(Err(message)) => app.notify(message),
                    Err(_) => app.notify("External editor stopped before returning.".to_owned()),
                }
                if events_closed || app.should_quit() {
                    break;
                }
            }
            maybe = session.next_input(), if external_edit.is_none() => {
                let (copy, notify_copy) = match maybe {
                    Ok(Some(Input::Event(Event::Key(key)))) => {
                        let (handled, command) = sessions.key(&mut app, key);
                        if sessions.picker.is_some() || sessions.pending {
                            transcript_search::cancel(&mut app);
                            path_completion::cancel(&mut app);
                        }
                        if let Some(command) = command
                            && let Err(error) =
                                requests.try_send(command, PendingRequest::Control)
                        {
                            sessions.pending = false;
                            if request_failed(
                                &mut app,
                                error,
                                "Session command queue is full; try again.",
                            ) {
                                return Ok(());
                            }
                        }
                        if handled {
                            // A picker key ends any local history search and
                            // stops background edge scrolling.
                            app.history.cancel_search();
                            app.text_selection.clear();
                            continue;
                        }
                        if !app.history.searching()
                            && !transcript_search::active(&app)
                            && !path_completion::active(&app)
                            && commands.key(&mut editor, key)
                        {
                            app.text_selection.clear();
                            continue;
                        }
                        let outcome = handle_key(&mut app, &mut editor, key);
                        if let Some(command) = outcome.command
                            && let Some((_command, request)) =
                                commands.dispatch(command, &mut app, &mut editor, &mut sessions)
                            && let Err(error) = requests.try_send(
                                request,
                                PendingRequest::Command {
                                    draft: editor.text(),
                                    session_id: app.session.as_ref().map(|s| s.id.clone()),
                                },
                            )
                        {
                            sessions.pending = false;
                            if request_failed(&mut app, error, "Command queue is full; draft kept.")
                            {
                                return Ok(());
                            }
                        }
                        if app.help_open {
                            app.history.cancel_search();
                        }
                        if outcome.cancel {
                            if let Some(association) = sessions.current_association() {
                                let command = SessionCommand::Cancel {
                                    session_id: association.session_id.clone(),
                                    task_id: association.task_id.clone(),
                                };
                                let pending = PendingRequest::Cancel {
                                    session_id: association.session_id,
                                    task_id: association.task_id,
                                };
                                match requests.try_send(command, pending) {
                                    Ok(_) => {
                                        app.cancelling = true;
                                        app.status = None;
                                    }
                                    Err(error) => {
                                        if request_failed(
                                            &mut app,
                                            error,
                                            "Cancel queue is full; task still running.",
                                        ) {
                                            return Ok(());
                                        }
                                    }
                                }
                            } else {
                                app.notify("No running task to cancel.".to_owned());
                            }
                        } else if let Some(message) = outcome.send {
                            let Some(metadata) = &app.session else { continue; };
                            if requests.has_pending_send(&metadata.id) {
                                app.notify("A message is already pending for this session.".to_owned());
                            } else {
                                let command = SessionCommand::Send {
                                    session_id: metadata.id.clone(),
                                    message: message.clone(),
                                };
                                let pending = PendingRequest::Send {
                                    session_id: metadata.id.clone(),
                                    message,
                                };
                                match requests.try_send(command, pending) {
                                    Ok(_) => app.status = Some("Sending…".to_owned()),
                                    Err(error) => {
                                        if request_failed(
                                            &mut app,
                                            error,
                                            "Send queue is full; draft kept.",
                                        ) {
                                            return Ok(());
                                        }
                                    }
                                }
                            }
                        }
                        if outcome.paste_clipboard {
                            handle_clipboard(&mut app, &mut editor);
                        }
                        if outcome.external_editor {
                            start_external_editor(
                                &mut session,
                                &mut app,
                                &editor,
                                &mut external_edit,
                            );
                        }
                        if outcome.quit {
                            break;
                        }
                        (outcome.copy, true)
                    }
                    Ok(Some(Input::Event(Event::Paste(text)))) => {
                        if sessions.picker.is_some() || sessions.pending {
                            app.text_selection.clear();
                            sessions.paste(&text);
                            continue;
                        }
                        handle_paste_text(&mut app, &mut editor, &text);
                        app.text_selection.clear();
                        (None, false)
                    }
                    Ok(Some(Input::Event(Event::Mouse(mouse)))) => {
                        if sessions.picker.is_some() || sessions.pending {
                            app.text_selection.clear();
                            continue;
                        }
                        if !app.history.searching()
                            && !transcript_search::active(&app)
                            && !path_completion::active(&app)
                            && commands.mouse(&mut editor, mouse)
                        {
                            app.text_selection.clear();
                            continue;
                        }
                        let position = ratatui::layout::Position::new(mouse.column, mouse.row);
                        if mouse.kind == MouseEventKind::Down(MouseButton::Left) && app.session_button_area.contains(position) {
                            // The global session picker closes any local search
                            // first so a switched session never caches it.
                            app.history.cancel_search();
                            transcript_search::cancel(&mut app);
                            path_completion::cancel(&mut app);
                            app.text_selection.clear();
                            sessions.pending = true;
                            if let Err(error) =
                                requests.try_send(SessionCommand::List, PendingRequest::Control)
                            {
                                sessions.pending = false;
                                if request_failed(
                                    &mut app,
                                    error,
                                    "Session command queue is full; try again.",
                                ) {
                                    return Ok(());
                                }
                            }
                            continue;
                        }
                        if path_completion::active(&app) {
                            app.text_selection.clear();
                            continue;
                        }
                        (handle_mouse(&mut app, mouse), false)
                    },
                    Ok(Some(Input::Theme(colors))) => {
                        app.set_theme(theme::native_theme(colors));
                        (None, false)
                    }
                    Ok(Some(Input::Event(Event::FocusLost))) => {
                        // A held drag must not survive focus loss even if the
                        // matching mouse-up never arrives.
                        app.text_selection.clear();
                        (None, false)
                    }
                    Ok(Some(Input::Event(_))) => (None, false),
                    Ok(None) => break,
                    Err(error) => return Err(error.into()),
                };
                if let Some(text) = copy {
                    match copy_to_clipboard(&text) {
                        Ok(()) if notify_copy => app.notify(format!("Copied {} characters", text.chars().count())),
                        Ok(()) => {}
                        Err(error) => app.notify(format!("Copy failed: {error}")),
                    }
                }
            }
            packet = events.recv(), if !events_closed => {
                let packet: crate::session::SessionEventPacket = match packet {
                    Some(packet) => packet,
                    None => {
                        if external_edit.is_some() {
                            events_closed = true;
                            continue;
                        }
                        break;
                    }
                };
                let request_id = packet.request_id;
                let resolution = request_id.and_then(|id| requests.take(id));
                let mut handled = false;
                match resolution {
                    Some(PendingRequest::Send { .. }) => {
                        let session_id = match &resolution {
                            Some(PendingRequest::Send { session_id, .. }) => session_id,
                            _ => unreachable!(),
                        };
                        match &packet.event {
                        SessionEvent::Accepted => {
                            if let Some(association) = packet.association.clone() {
                                if let Some(PendingRequest::Send { message, .. }) = &resolution {
                                    sessions.accept_send(&mut app, &mut editor, association, message);
                                }
                            } else {
                                sessions.notify_for(
                                    &mut app,
                                    session_id.as_str(),
                                    "Send accepted without task association.".to_owned(),
                                );
                            }
                            if app
                                .session
                                .as_ref()
                                .is_some_and(|active| active.id == *session_id)
                            {
                                app.status = None;
                            }
                            handled = true;
                        }
                        SessionEvent::Error { operation, error } => {
                            if app
                                .session
                                .as_ref()
                                .is_some_and(|active| active.id == *session_id)
                            {
                                app.status = None;
                            }
                            sessions.notify_for(
                                &mut app,
                                session_id.as_str(),
                                format!("{}: {}", operation, error.message),
                            );
                            handled = true;
                        }
                        _ => {}
                        }
                    }
                    Some(PendingRequest::Command {
                        draft,
                        session_id,
                    }) => {
                        sessions.pending = false;
                        match &packet.event {
                            SessionEvent::Error { operation, error } => {
                                let message = format!("{}: {}", operation, error.message);
                                if let Some(session_id) = session_id {
                                    sessions.notify_for(&mut app, session_id.as_str(), message);
                                } else {
                                    app.notify(message);
                                }
                                handled = true;
                            }
                            _ => {
                                let target_current = session_id.as_ref().is_none_or(|session_id| {
                                    app
                                        .session
                                        .as_ref()
                                        .is_some_and(|active| active.id == *session_id)
                                });
                                if target_current {
                                    if editor.text() == draft {
                                        editor.reset();
                                    }
                                } else if let Some(session_id) = session_id {
                                    sessions.reset_cached_editor_if(session_id.as_str(), draft.as_str());
                                }
                            }
                        }
                    }
                    Some(PendingRequest::Control) => {
                        sessions.pending = false;
                        if let SessionEvent::Error { operation, error } = &packet.event {
                            app.notify(format!("{}: {}", operation, error.message));
                            handled = true;
                        }
                    }
                    Some(PendingRequest::Cancel {
                        session_id,
                        task_id,
                    }) => match &packet.event {
                        SessionEvent::Cancelled { status } => {
                            let current = sessions.current_association();
                            let matches = current.as_ref().is_some_and(|current| {
                                current.session_id == session_id && current.task_id == task_id
                            });
                            if matches && *status == "not_running" {
                                app.cancelling = false;
                                app.notify("No running task to cancel.".to_owned());
                            }
                            handled = true;
                        }
                        SessionEvent::Error { operation, error } => {
                            let current = sessions.current_association();
                            let matches = current.as_ref().is_some_and(|current| {
                                current.session_id == session_id && current.task_id == task_id
                            });
                            if matches {
                                app.cancelling = false;
                            }
                            sessions.notify_for(
                                &mut app,
                                session_id.as_str(),
                                format!("{}: {}", operation, error.message),
                            );
                            handled = true;
                        }
                        _ => {}
                    },
                    None => {}
                }
                if !handled {
                    let association = packet.association.clone();
                    let event = packet.event;
                    sessions.apply_event_with_association(&mut app, &mut editor, event, association);
                }
            }
            _ = tokio::time::sleep_until(feedback_deadline.unwrap_or_else(tokio::time::Instant::now)), if feedback_deadline.is_some() => {
                app.expire_feedback(std::time::Instant::now());
            }
            _ = tokio::time::sleep_until(scroll_deadline.unwrap_or_else(tokio::time::Instant::now)), if scroll_deadline.is_some() => {
                view::selection_scroll_tick(&mut app);
            }
        }
    }
    Ok(())
}

fn request_failed(app: &mut App, error: RequestSendError, full_message: &str) -> bool {
    match error {
        RequestSendError::Full => app.notify(full_message.to_owned()),
        RequestSendError::Closed => app.notify("Session output is closed.".to_owned()),
    }
    matches!(error, RequestSendError::Closed)
}

#[derive(Default)]
pub struct KeyOutcome {
    pub send: Option<UserInput>,
    pub command: Option<Command>,
    pub copy: Option<String>,
    /// Ctrl+V intent; the run loop performs the actual clipboard IO.
    pub paste_clipboard: bool,
    /// Esc intent while the agent is running; the run loop sends Cancel.
    pub cancel: bool,
    pub quit: bool,
    pub external_editor: bool,
}

/// Maps one key event to state changes. Kept free of terminal I/O so it can be
/// driven directly by tests. Held keys arrive as `Repeat` and are accepted;
/// `Release` is ignored.
pub fn handle_key(app: &mut App, editor: &mut Editor, key: KeyEvent) -> KeyOutcome {
    let mut outcome = KeyOutcome {
        send: None,
        command: None,
        copy: None,
        paste_clipboard: false,
        cancel: false,
        quit: false,
        external_editor: false,
    };
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return outcome;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let had_selection = app.text_selection.active();

    // A rendered-transcript search is modal: it owns every key (Esc/Ctrl+C
    // close it before any cancel/clear), and only Ctrl+V reaches clipboard IO.
    if transcript_search::active(app) {
        return transcript_search_key(app, key);
    }

    // The path popup owns only its navigation/confirm keys; any other key
    // dismisses it and continues through the normal map (typing edits normally).
    if path_completion::active(app) {
        let plain = !ctrl && !alt;
        match key.code {
            KeyCode::Esc if plain => {
                path_completion::cancel(app);
                return outcome;
            }
            KeyCode::Up if plain => {
                path_completion::move_selection(app, false);
                return outcome;
            }
            KeyCode::Down if plain => {
                path_completion::move_selection(app, true);
                return outcome;
            }
            KeyCode::Tab | KeyCode::Enter if plain => {
                path_completion::confirm(app, editor);
                return outcome;
            }
            _ => path_completion::cancel(app),
        }
    }

    if ctrl
        && matches!(key.code, KeyCode::Char('c' | 'C'))
        && let Some(text) = app.text_selection.text()
    {
        outcome.copy = Some(text);
        app.text_selection.clear();
        return outcome;
    }
    app.text_selection.clear();

    if ctrl && matches!(key.code, KeyCode::Char('v' | 'V')) {
        app.help_open = false;
        if app.log_view.is_some() {
            log::close(app);
        }
        app.focus = Focus::Input;
        outcome.paste_clipboard = true;
        return outcome;
    }

    // A local prompt-history search is modal in the input focus: it consumes
    // every key (Esc/Ctrl+C included), so the hidden composer is never edited.
    if app.history.searching() && app.focus == Focus::Input {
        // F1 is global help: it closes the search first.
        if key.code == KeyCode::F(1) && !ctrl && !alt {
            app.history.cancel_search();
            app.toggle_help();
            return outcome;
        }
        return history_search_key(app, editor, key);
    }

    if key.code == KeyCode::F(1) && !ctrl && !alt {
        app.toggle_help();
        return outcome;
    }
    if app.help_open {
        match key.code {
            KeyCode::Esc => app.help_open = false,
            KeyCode::Char('c') if ctrl => app.help_open = false,
            KeyCode::Up => help::scroll(app, -1),
            KeyCode::Down => help::scroll(app, 1),
            KeyCode::PageUp => help::scroll(app, -(app.help_area.height.max(1) as isize)),
            KeyCode::PageDown => help::scroll(app, app.help_area.height.max(1) as isize),
            KeyCode::Home => app.help_scroll = 0,
            KeyCode::End => help::scroll(app, isize::MAX),
            _ => {}
        }
        return outcome;
    }
    if key.code == KeyCode::Char('?')
        && !ctrl
        && !alt
        && app.focus == Focus::Input
        && app.log_view.is_none()
        && editor.is_empty()
    {
        app.toggle_help();
        return outcome;
    }

    if ctrl && key.code == KeyCode::Char('c') {
        outcome.quit = handle_ctrl_c(app, editor);
        return outcome;
    }

    // Conversation start/end from any focus; the log sub-view gives way first.
    if ctrl && key.code == KeyCode::Home {
        if app.log_view.is_some() {
            log::close(app);
        }
        app.home();
        return outcome;
    }
    if ctrl && key.code == KeyCode::End {
        if app.log_view.is_some() {
            log::close(app);
        }
        app.end();
        return outcome;
    }

    // Alt+Up/Down scrolls one line in any focus, including the log sub-view.
    if alt {
        match key.code {
            KeyCode::Up => {
                if app.log_view.is_some() {
                    view::log_scroll(app, -1);
                } else {
                    view::page_up(app, 1);
                }
                return outcome;
            }
            KeyCode::Down => {
                if app.log_view.is_some() {
                    view::log_scroll(app, 1);
                } else {
                    view::page_down(app, 1);
                }
                return outcome;
            }
            _ => {}
        }
    }

    if app.log_view.is_some() {
        match key.code {
            KeyCode::Esc => log::close(app),
            KeyCode::Down => view::log_scroll(app, 1),
            KeyCode::Up => view::log_scroll(app, -1),
            KeyCode::PageDown => view::log_page_nav(app, true),
            KeyCode::PageUp => view::log_page_nav(app, false),
            _ => {}
        }
        return outcome;
    }

    // F3 opens rendered-transcript search; Ctrl+F stays character-right.
    if !ctrl && !alt && key.code == KeyCode::F(3) {
        path_completion::cancel(app);
        transcript_search::open(app);
        return outcome;
    }

    // Ctrl+Shift+Up/Down jumps between real user-message boundaries. Both
    // modifiers are required so plain/Alt line scrolling and editor movement
    // stay untouched, and local overlays/log keep precedence above.
    if ctrl && shift {
        match key.code {
            KeyCode::Up => {
                view::previous_user(app);
                return outcome;
            }
            KeyCode::Down => {
                view::next_user(app);
                return outcome;
            }
            _ => {}
        }
    }

    if key.code == KeyCode::Esc && !ctrl && !alt {
        // Local UI first: Esc clears an active selection before agent work.
        if had_selection {
            app.text_selection.clear();
            return outcome;
        }
        if app.running && !app.cancelling {
            outcome.cancel = true;
            return outcome;
        }
        if app.focus == Focus::Transcript {
            app.focus = Focus::Input;
        }
        return outcome;
    }

    let page = app.last_height.saturating_sub(1).max(1);
    match key.code {
        KeyCode::PageUp => {
            view::page_up(app, page);
            return outcome;
        }
        KeyCode::PageDown => {
            view::page_down(app, page);
            return outcome;
        }
        KeyCode::Tab => {
            if app.focus == Focus::Input && path_completion::tab(app, editor) {
                return outcome;
            }
            match app.focus {
                Focus::Input => view::focus_transcript(app),
                Focus::Transcript => app.focus = Focus::Input,
            }
            return outcome;
        }
        _ => {}
    }

    match app.focus {
        Focus::Transcript => {
            match key.code {
                KeyCode::Up if shift => view::select(app, -1),
                KeyCode::Down if shift => view::select(app, 1),
                KeyCode::Up => view::page_up(app, 1),
                KeyCode::Down => view::page_down(app, 1),
                KeyCode::Home => app.home(),
                KeyCode::End => app.end(),
                KeyCode::Enter => {
                    app.toggle_selected();
                }
                KeyCode::Char('y') => outcome.copy = view::copy_source(app),
                KeyCode::Char('Y') => outcome.copy = view::copy_code_source(app),
                KeyCode::Char('l') => {
                    log::open(app);
                }
                _ => {}
            }
            outcome
        }
        // Editing and history keys only apply to the input focus.
        Focus::Input => input_key(app, editor, key),
    }
}

/// Input-focus key map. Kept separate so record/log bindings cannot regress.
fn input_key(app: &mut App, editor: &mut Editor, key: KeyEvent) -> KeyOutcome {
    let mut outcome = KeyOutcome {
        send: None,
        command: None,
        copy: None,
        paste_clipboard: false,
        cancel: false,
        quit: false,
        external_editor: false,
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    if app.history.searching() {
        return history_search_key(app, editor, key);
    }
    match key.code {
        KeyCode::Enter if alt || shift => editor.insert_newline(),
        KeyCode::Enter => {
            if !editor.is_empty() {
                match command::parse(&editor.text(), !editor.images().is_empty()) {
                    Ok(ParsedInput::Command(command)) => outcome.command = Some(command),
                    Ok(ParsedInput::Message(text)) if !app.running => {
                        let mut message = editor.draft_message();
                        message.text = text;
                        outcome.send = Some(message);
                    }
                    Ok(ParsedInput::Message(_)) => {
                        app.status =
                            Some("Draft kept. Send again when the task finishes.".to_owned())
                    }
                    Err(error) => app.notify(error.to_string()),
                }
            }
        }
        KeyCode::Char('j') if ctrl => editor.insert_newline(),
        KeyCode::Char('b') if ctrl => editor.move_left(),
        KeyCode::Char('f') if ctrl => editor.move_right(),
        KeyCode::Char('b') if alt => editor.move_word_left(),
        KeyCode::Char('f') if alt => editor.move_word_right(),
        KeyCode::Char('a') if ctrl => editor.move_line_start(),
        KeyCode::Char('e') if ctrl => editor.move_line_end(),
        KeyCode::Char('w') if ctrl => editor.delete_word_backward(),
        KeyCode::Char('d') if alt => editor.delete_word_forward(),
        KeyCode::Char('d') if ctrl => {
            if editor.is_empty() {
                outcome.quit = exit_or_wait(app);
            } else {
                editor.delete();
            }
        }
        KeyCode::Char('u') if ctrl => editor.kill_to_line_start(),
        KeyCode::Char('k') if ctrl => editor.kill_to_line_end(),
        KeyCode::Char('y') if ctrl => editor.yank(),
        KeyCode::Char('h') if ctrl => editor.backspace(),
        KeyCode::Char('p') if ctrl => navigate_or_move(app, editor, false),
        KeyCode::Char('n') if ctrl => navigate_or_move(app, editor, true),
        KeyCode::Char('r') if ctrl => app.history.open_or_older(),
        KeyCode::Char('g') if ctrl => outcome.external_editor = true,
        KeyCode::Char('u') if alt => {
            editor.undo();
        }
        KeyCode::Char('r') if alt => {
            editor.redo();
        }
        KeyCode::Left if ctrl || alt => editor.move_word_left(),
        KeyCode::Right if ctrl || alt => editor.move_word_right(),
        KeyCode::Left => editor.move_left(),
        KeyCode::Right => editor.move_right(),
        KeyCode::Backspace if ctrl || alt => editor.delete_word_backward(),
        KeyCode::Backspace => editor.backspace(),
        KeyCode::Delete if ctrl || alt => editor.delete_word_forward(),
        KeyCode::Delete => editor.delete(),
        KeyCode::Up => navigate_or_move(app, editor, false),
        KeyCode::Down => navigate_or_move(app, editor, true),
        KeyCode::Home => editor.home(),
        KeyCode::End => editor.end(),
        // Alt and Ctrl letters are shortcuts; they must not be inserted.
        KeyCode::Char(character) if !ctrl && !alt => editor.insert_typed(character),
        _ => {}
    }
    outcome
}

/// Up/Down (and Ctrl+P/N) share visual movement and history recall. History is
/// used only when the draft is empty, or still equals the recalled entry with
/// the cursor at the document boundary.
fn navigate_or_move(app: &mut App, editor: &mut Editor, forward: bool) {
    // Compare fields instead of building a full draft so large base64 snapshots
    // are never cloned on every Up/Down keypress.
    let empty = editor.text().is_empty() && editor.images().is_empty();
    let navigating = if empty {
        !app.history.is_empty()
    } else if let Some(recalled) = app.history.recalled() {
        editor.text() == recalled.text
            && editor.images() == recalled.images.as_slice()
            && editor.at_document_boundary()
    } else {
        false
    };
    if !navigating {
        if forward {
            editor.move_visual_down(app.input_width);
        } else {
            editor.move_visual_up(app.input_width);
        }
        return;
    }
    if forward {
        let was = app.history.navigating();
        match app.history.newer() {
            Some(entry) => editor.set_draft(entry),
            None if was => editor.clear(),
            None => {}
        }
    } else if let Some(entry) = app.history.older() {
        editor.set_draft(entry);
    }
}

/// Local prompt-history search keys. The composer draft is untouched here;
/// Enter clones the chosen entry into the editor and never sends. Esc and
/// Ctrl+C cancel and leave the draft and ordinary navigation unchanged.
fn history_search_key(app: &mut App, editor: &mut Editor, key: KeyEvent) -> KeyOutcome {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Esc => app.history.cancel_search(),
        KeyCode::Char('c') if ctrl => app.history.cancel_search(),
        KeyCode::Char('r') if ctrl => app.history.move_older(),
        KeyCode::Up => app.history.move_older(),
        KeyCode::Char('p') if ctrl => app.history.move_older(),
        KeyCode::Down => app.history.move_newer(),
        KeyCode::Char('n') if ctrl => app.history.move_newer(),
        KeyCode::Backspace => app.history.query_backspace(),
        KeyCode::Enter => {
            if let Some(entry) = app.history.accept_search() {
                editor.set_draft(entry);
            }
        }
        KeyCode::Char(character) if !ctrl && !alt => app.history.query_push(character),
        _ => {}
    }
    KeyOutcome::default()
}

/// Modal rendered-transcript search key map. The composer and its history are
/// never edited here; Ctrl+V only requests clipboard IO for query text.
fn transcript_search_key(app: &mut App, key: KeyEvent) -> KeyOutcome {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let mut outcome = KeyOutcome::default();
    match key.code {
        KeyCode::Esc => transcript_search::cancel(app),
        KeyCode::Char('c') if ctrl => transcript_search::cancel(app),
        KeyCode::Char('v') if ctrl => outcome.paste_clipboard = true,
        KeyCode::F(1) if !ctrl && !alt => {
            transcript_search::cancel(app);
            app.toggle_help();
        }
        KeyCode::F(3) => transcript_search::step(app, !shift),
        KeyCode::Enter | KeyCode::Down => transcript_search::step(app, true),
        KeyCode::Up => transcript_search::step(app, false),
        KeyCode::Backspace => transcript_search::backspace(app),
        KeyCode::Char('h') if ctrl => transcript_search::backspace(app),
        KeyCode::Char(character) if !ctrl && !alt => transcript_search::push_char(app, character),
        _ => {}
    }
    outcome
}

/// Ctrl+C: close the log sub-view first, else save and clear a non-empty draft
/// (even while busy, without requesting exit), else use the shared exit path.
fn handle_ctrl_c(app: &mut App, editor: &mut Editor) -> bool {
    if app.log_view.is_some() {
        log::close(app);
        return false;
    }
    if !editor.is_empty() {
        app.history.push(editor.clear_for_history());
        app.focus = Focus::Input;
        app.status = None;
        return false;
    }
    exit_or_wait(app)
}

/// The one exit path shared by Ctrl+C and Ctrl+D on an empty input: quit when
/// idle, otherwise ask the worker to finish and wait truthfully.
fn exit_or_wait(app: &mut App) -> bool {
    if app.running {
        app.quit_after_turn = true;
        app.status = Some("Waiting for the current task before exiting.".to_owned());
        false
    } else {
        true
    }
}

/// Footer actions and help scrolling are handled before transcript routing.
/// Dragging copies visible text; navigation never edits the draft or its focus.
/// Returns text for the event loop to write to the clipboard.
pub fn handle_mouse(app: &mut App, mouse: MouseEvent) -> Option<String> {
    if transcript_search::active(app) {
        let position = ratatui::layout::Position::new(mouse.column, mouse.row);
        if mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && app.help_button_area.contains(position)
        {
            transcript_search::cancel(app);
            app.toggle_help();
        }
        return None;
    }
    // A keyboard-owned history search consumes ordinary mouse events; only a
    // Help click is global and closes the search first.
    if app.history.searching() {
        let position = ratatui::layout::Position::new(mouse.column, mouse.row);
        if mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && app.help_button_area.contains(position)
        {
            app.history.cancel_search();
            app.toggle_help();
        }
        return None;
    }
    let position = ratatui::layout::Position::new(mouse.column, mouse.row);
    if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
        app.text_selection.clear();
        app.disclosure_press = None;
        if app.help_button_area.contains(position) {
            app.toggle_help();
        } else if !app.help_open && app.latest_button_area.contains(position) {
            if app.log_view.is_some() {
                log::close(app);
            }
            app.end();
        } else {
            if !app.help_open && app.log_view.is_none() {
                app.disclosure_press = app
                    .disclosure_buttons
                    .iter()
                    .find(|(area, _)| area.contains(position))
                    .map(|(_, index)| *index);
            }
            let transcript =
                !app.help_open && app.log_view.is_none() && app.transcript_area.contains(position);
            // Pin the mapped viewport at mouse-down so a potential drag cannot
            // be disturbed by new output; a click without a drag restores
            // follow on release.
            let was_following = app.follow;
            if transcript && was_following {
                view::page_up(app, 0);
            }
            let started = transcript && {
                let viewport = app.transcript_rows.clone();
                app.text_selection.start_transcript(
                    app.transcript_area,
                    position,
                    &viewport,
                    was_following,
                )
            };
            if !started {
                if was_following {
                    app.end();
                }
                app.text_selection.start(position);
            }
        }
        return None;
    }
    match mouse.kind {
        MouseEventKind::Drag(MouseButton::Left) => {
            app.disclosure_press = None;
            if app.text_selection.is_transcript() {
                if app.follow {
                    view::page_up(app, 0);
                }
                view::selection_drag(app, position);
            } else {
                app.text_selection.drag(position);
            }
            return None;
        }
        MouseEventKind::Up(MouseButton::Left) => {
            let pressed = app.disclosure_press.take();
            let restore_follow = app.text_selection.take_follow_restore();
            let viewport = app.transcript_rows.clone();
            let area = app.transcript_area;
            let copied = app.text_selection.finish(position, area, &viewport);
            if copied.is_none() && !app.help_open && app.log_view.is_none() {
                // Freeze the viewport before reflow so a long thought, tool or
                // activity detail opens from its beginning.
                let toggled = pressed.is_some_and(|index| {
                    app.disclosure_buttons
                        .iter()
                        .any(|(area, record)| *record == index && area.contains(position))
                });
                if let Some(index) = pressed.filter(|_| toggled) {
                    view::page_up(app, 0);
                    app.toggle_record(index);
                    app.text_selection.clear();
                } else if restore_follow {
                    // A click without a drag never permanently leaves follow.
                    app.end();
                }
                // A same-cell click is not a selection; never let an invisible
                // zero-size selection shadow Esc.
                app.text_selection.clear();
            }
            return copied;
        }
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            app.disclosure_press = None;
            app.text_selection.clear();
        }
        _ => {}
    }
    if app.help_open {
        if app.help_area.contains(position) {
            match mouse.kind {
                MouseEventKind::ScrollUp => help::scroll(app, -3),
                MouseEventKind::ScrollDown => help::scroll(app, 3),
                _ => {}
            }
        }
        return None;
    }
    let over_pane = app.transcript_area.contains(position);
    if !over_pane {
        return None;
    }
    match mouse.kind {
        MouseEventKind::ScrollUp => {
            if app.log_view.is_some() {
                view::log_scroll(app, -3);
            } else {
                view::page_up(app, 3);
            }
        }
        MouseEventKind::ScrollDown => {
            if app.log_view.is_some() {
                view::log_scroll(app, 3);
            } else {
                view::page_down(app, 3);
            }
        }
        _ => {}
    }
    None
}

#[cfg(test)]
#[path = "../../../tests/tui/mod.rs"]
mod tests;

fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|error| error.to_string())?;
    clipboard.set_text(text).map_err(|error| error.to_string())
}

/// Terminal paste or drag: load a single image path, otherwise insert text.
/// A failed explicit image path keeps the original paste as editable text and
/// never clears existing attachments.
fn handle_paste_text(app: &mut App, editor: &mut Editor, text: &str) {
    if transcript_search::active(app) {
        transcript_search::push_str(app, text);
        return;
    }
    app.help_open = false;
    if app.log_view.is_some() {
        log::close(app);
    }
    app.focus = Focus::Input;
    // Paste during search edits only the query; the hidden draft is untouched.
    if app.history.searching() {
        app.history.query_paste(text);
        return;
    }
    match message::classify_image_paste(text, |path| path.is_file()) {
        Some(paste) => {
            if let Err(error) = attach_image(app, editor, paste) {
                app.status = Some(error);
                editor.insert_str(text);
            }
        }
        None => editor.insert_str(text),
    }
}

/// Ctrl+G: suspend the same terminal, run the configured editor with the draft
/// in a temp file, restore the terminal, then load changed UTF-8 text as one
/// undoable edit. Failures only notify and leave the exact draft/cursor/history.
type ExternalEdit = (
    tokio::sync::oneshot::Receiver<Result<String, String>>,
    Option<external_editor::SigintGuard>,
    String,
);

fn start_external_editor(
    session: &mut terminal::TerminalSession,
    app: &mut App,
    editor: &Editor,
    external_edit: &mut Option<ExternalEdit>,
) {
    if external_edit.is_some() {
        return;
    }
    let launch = match external_editor::from_env() {
        Ok(launch) => launch,
        Err(message) => {
            app.notify(message);
            return;
        }
    };
    let Some(session_id) = app.session.as_ref().map(|session| session.id.clone()) else {
        app.notify("Session is not ready. Draft kept.".to_owned());
        return;
    };
    let guard = match external_editor::SigintGuard::install() {
        Ok(guard) => guard,
        Err(message) => {
            app.notify(message);
            return;
        }
    };
    if let Err(error) = session.suspend_external() {
        let _ = session.resume_external();
        drop(guard);
        app.notify(format!(
            "External editor could not suspend the terminal: {error}"
        ));
        return;
    }
    let original = editor.text();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let result = external_editor::edit(&launch, &original);
        let _ = sender.send(result);
    });
    *external_edit = Some((receiver, Some(guard), session_id));
}

#[cfg(test)]
fn load_external_text(app: &mut App, editor: &mut Editor, result: Result<String, String>) {
    match result {
        Ok(text) => {
            editor.replace_text(&text);
        }
        Err(message) => app.notify(message),
    }
}

/// Ctrl+V: clipboard image first, else clipboard text (which may itself be an
/// image path).
fn handle_clipboard(app: &mut App, editor: &mut Editor) {
    let mut clipboard = match arboard::Clipboard::new() {
        Ok(clipboard) => clipboard,
        Err(error) => {
            app.status = Some(format!("Clipboard unavailable: {error}"));
            return;
        }
    };
    // While a transcript search is open, text paste edits only the query and
    // an image clipboard is reported without touching the preserved draft.
    if transcript_search::active(app) {
        match clipboard.get_image() {
            Ok(_) => app.notify("Close transcript search before attaching an image.".to_owned()),
            Err(_) => match clipboard.get_text() {
                Ok(text) => transcript_search::push_str(app, &text),
                Err(error) => app.notify(format!("Clipboard read failed: {error}")),
            },
        }
        return;
    }
    // While searching, text paste edits only the query and an image clipboard
    // is reported without touching the preserved draft or history images.
    if app.history.searching() {
        match clipboard.get_image() {
            Ok(_) => app.notify("Close history search before attaching an image.".to_owned()),
            Err(_) => match clipboard.get_text() {
                Ok(text) => app.history.query_paste(&text),
                Err(error) => app.notify(format!("Clipboard read failed: {error}")),
            },
        }
        return;
    }
    match clipboard.get_image() {
        Ok(image) => match encode_clipboard_png(&image) {
            Ok(bytes) => {
                editor.insert_image(ImageAttachment::from_bytes(
                    "clipboard.png",
                    "image/png",
                    &bytes,
                ));
                app.status = None;
            }
            Err(error) => app.status = Some(error),
        },
        Err(_) => match clipboard.get_text() {
            Ok(text) => handle_paste_text(app, editor, &text),
            Err(error) => app.status = Some(format!("Clipboard read failed: {error}")),
        },
    }
}

fn attach_image(app: &mut App, editor: &mut Editor, path: PathBuf) -> Result<(), String> {
    let path = expand_home(&path);
    let bytes = std::fs::read(&path).map_err(|error| format!("Could not read image: {error}"))?;
    let mime = message::image_mime_from_bytes(&bytes)
        .ok_or_else(|| "Unsupported image format".to_owned())?;
    // Decode once so truncated or corrupt files are rejected here instead of
    // being sent as a broken data URL.
    image::load_from_memory(&bytes).map_err(|error| format!("Could not decode image: {error}"))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("image")
        .to_owned();
    editor.insert_image(ImageAttachment::from_bytes(name, mime, &bytes));
    app.status = None;
    Ok(())
}

/// Expands a leading `~/` (the only shell form we accept) so drag paths can be
/// opened relative to the user's home directory.
fn expand_home(path: &std::path::Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    path.to_path_buf()
}

fn encode_clipboard_png(image: &arboard::ImageData<'_>) -> Result<Vec<u8>, String> {
    let buffer = image::RgbaImage::from_raw(
        image.width as u32,
        image.height as u32,
        image.bytes.to_vec(),
    )
    .ok_or_else(|| "Invalid clipboard image dimensions".to_owned())?;
    let mut cursor = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(buffer)
        .write_to(&mut cursor, image::ImageFormat::Png)
        .map_err(|error| format!("Could not encode image: {error}"))?;
    Ok(cursor.into_inner())
}
