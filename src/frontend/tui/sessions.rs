use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use unicode_segmentation::UnicodeSegmentation;

use super::{
    editor::Editor,
    state::{App, Record},
    textwrap, theme,
};
use crate::{
    client::{AgentEvent, Association, SessionSummary, history::TaskStatus},
    session::{SessionCommand, SessionEvent},
};

pub struct Picker {
    pub sessions: Vec<SessionSummary>,
    /// Indices into `sessions` matching the active query, in core order.
    /// `selected` indexes this list, so navigation only visits visible results.
    visible: Vec<usize>,
    /// Index into `visible`.
    pub selected: usize,
    active_id: Option<String>,
    rename: Option<Rename>,
    /// `Some` while the local search field is active; `None` in browse mode.
    query: Option<String>,
}

/// An in-progress rename. The target ID is pinned so an asynchronous title
/// update cannot redirect the rename to a different result.
struct Rename {
    id: String,
    title: String,
}

impl Picker {
    /// The session a confirm or rename would act on; `None` when the query has
    /// no matches.
    fn selected_session(&self) -> Option<&SessionSummary> {
        self.visible
            .get(self.selected)
            .and_then(|index| self.sessions.get(*index))
    }

    /// Recomputes matches. `preferred` stays selected when it still matches,
    /// otherwise the first result is selected.
    fn refilter(&mut self, preferred: Option<&str>) {
        self.visible = match &self.query {
            Some(query) => self
                .sessions
                .iter()
                .enumerate()
                .filter(|(_, session)| session_matches(session, query))
                .map(|(index, _)| index)
                .collect(),
            None => (0..self.sessions.len()).collect(),
        };
        self.selected = preferred
            .and_then(|id| {
                self.visible
                    .iter()
                    .position(|index| self.sessions[*index].metadata.id == id)
            })
            .unwrap_or(0);
    }

    /// Recomputes matches while keeping the selected session when it still
    /// matches.
    fn refresh(&mut self) {
        let preferred = self
            .selected_session()
            .map(|session| session.metadata.id.clone());
        self.refilter(preferred.as_deref());
    }
}

/// Case-insensitive, whitespace-separated AND match over the metadata the
/// picker already holds: title, ID, working directory and model route.
fn session_matches(session: &SessionSummary, query: &str) -> bool {
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if terms.is_empty() {
        return true;
    }
    let title = session.metadata.title.to_lowercase();
    let id = session.metadata.id.to_lowercase();
    let directory = session
        .metadata
        .working_directory
        .to_string_lossy()
        .to_lowercase();
    let model = session.metadata.model.to_lowercase();
    terms.iter().all(|term| {
        title.contains(term)
            || id.contains(term)
            || directory.contains(term)
            || model.contains(term)
    })
}

/// Removes the last grapheme so search Backspace is safe for combining marks
/// and ZWJ sequences.
fn pop_last_grapheme(text: &mut String) {
    if let Some((index, _)) = text.grapheme_indices(true).next_back() {
        text.truncate(index);
    }
}

/// One non-current session projection. `association`/`finished` keep task
/// identity so late events cannot cross tasks and a finished task is not
/// finalized twice.
struct OpenedData {
    metadata: crate::client::SessionMetadata,
    replay: crate::client::history::History,
    context: AgentEvent,
    state: crate::client::SessionState,
    stream: Option<Association>,
}

struct CachedSession {
    app: App,
    editor: Editor,
    association: Option<Association>,
    finished: Option<String>,
    canonical: Option<String>,
}

#[derive(Default)]
pub struct Sessions {
    pub picker: Option<Picker>,
    pub pending: bool,
    cached: HashMap<String, CachedSession>,
    /// Current projection's observed active association.
    association: Option<Association>,
    /// Current projection's most recently finalized task.
    finished: Option<String>,
    /// Current projection was canonically repaired for this task before its
    /// terminal packet arrived.
    canonical: Option<String>,
}

impl Sessions {
    pub fn reset_cached_editor_if(&mut self, session_id: &str, draft: &str) {
        if let Some(cached) = self.cached.get_mut(session_id)
            && cached.editor.text() == draft
        {
            cached.editor.reset();
        }
    }

    pub fn replace_cached_text(&mut self, session_id: &str, text: &str) {
        if let Some(cached) = self.cached.get_mut(session_id) {
            cached.editor.replace_text(text);
        }
    }

    pub fn notify_for(&mut self, app: &mut App, session_id: &str, message: String) {
        if app
            .session
            .as_ref()
            .is_some_and(|active| active.id == session_id)
        {
            app.notify(message);
        } else if let Some(cached) = self.cached.get_mut(session_id) {
            cached.app.notify(message);
        }
    }

    pub fn current_association(&self) -> Option<Association> {
        self.association.clone()
    }

    /// Applies a Runtime acceptance reply. The draft is only cleared when the
    /// target projection still holds the submitted message.
    pub fn accept_send(
        &mut self,
        app: &mut App,
        editor: &mut Editor,
        association: Association,
        message: &crate::input::UserInput,
    ) {
        if app
            .session
            .as_ref()
            .is_some_and(|active| active.id == association.session_id)
        {
            if editor.matches_message(message) {
                let _ = editor.take();
            }
            app.submit(message.clone());
            self.association = Some(association);
            self.finished = None;
        } else if let Some(cached) = self.cached.get_mut(&association.session_id) {
            if cached.editor.matches_message(message) {
                let _ = cached.editor.take();
            }
            cached.app.submit(message.clone());
            cached.association = Some(association);
            cached.finished = None;
        }
    }

    /// Applies an owned service event while the packet that carried it is still
    /// alive in the caller. `association` is the packet's authoritative task
    /// envelope (authoritative over any per-event session string).
    #[cfg(test)]
    pub fn apply_event(&mut self, app: &mut App, editor: &mut Editor, event: SessionEvent) {
        self.apply_event_with_association(app, editor, event, None);
    }

    pub fn apply_event_with_association(
        &mut self,
        app: &mut App,
        editor: &mut Editor,
        event: SessionEvent,
        association: Option<Association>,
    ) {
        let renamed = matches!(&event, SessionEvent::Renamed(_));
        match event {
            SessionEvent::Opened {
                metadata,
                replay,
                context,
                state,
                stream,
            } => self.open(
                app,
                editor,
                OpenedData {
                    metadata,
                    replay,
                    context,
                    state,
                    stream,
                },
            ),
            SessionEvent::HistoryRestored {
                metadata,
                replay,
                context,
                state,
            } => self.history_restored(app, metadata, replay, context, state, association),
            SessionEvent::Listed(sessions) => {
                let active_id = app.session.as_ref().map(|session| session.id.clone());
                let mut picker = Picker {
                    sessions,
                    visible: Vec::new(),
                    selected: 0,
                    active_id: active_id.clone(),
                    rename: None,
                    query: None,
                };
                picker.refilter(active_id.as_deref());
                self.picker = Some(picker);
                self.pending = false;
                app.help_open = false;
                app.text_selection.clear();
            }
            SessionEvent::Renamed(metadata) | SessionEvent::TitleUpdated(metadata) => {
                self.metadata_update(app, metadata, renamed);
            }
            SessionEvent::Agent { session_id, event } => {
                self.agent(app, session_id, event, association);
            }
            SessionEvent::Accepted => {}
            SessionEvent::Finished(finished) => self.finished(app, finished, association),
            SessionEvent::Cancelled { status } => {
                if status == "not_running"
                    && let Some(association) = association
                    && self
                        .association
                        .as_ref()
                        .is_some_and(|current| current.task_id == association.task_id)
                {
                    app.cancelling = false;
                    app.notify("No running task to cancel.".to_owned());
                }
            }
            SessionEvent::Error { operation, error } => {
                let message = format!("{operation}: {}", error.message);
                if let Some(association) = association {
                    self.notify_for(app, &association.session_id, message);
                } else {
                    app.notify(message);
                }
            }
        }
    }

    fn open(&mut self, app: &mut App, editor: &mut Editor, data: OpenedData) {
        let OpenedData {
            metadata,
            replay,
            context,
            state,
            stream,
        } = data;
        let target = metadata.id.clone();
        let current_theme = app.theme;
        let same_current = app
            .session
            .as_ref()
            .is_some_and(|active| active.id == target);
        let old_association = self.association.clone();
        let old_finished = self.finished.clone();
        let old_canonical = self.canonical.clone();
        let cached_target = self.cached.remove(&target);
        let target_association = if same_current {
            old_association.as_ref()
        } else {
            cached_target
                .as_ref()
                .and_then(|cached| cached.association.as_ref())
        };
        let live = stream.as_ref().is_some_and(|stream| {
            target_association.is_some_and(|current| same_association(current, stream))
        });
        let (
            mut next,
            mut next_editor,
            _selected_association,
            selected_finished,
            selected_canonical,
        ) = if same_current {
            let next = if live {
                std::mem::replace(app, App::new())
            } else {
                let mut next = App::restore(&replay);
                merge_view(&mut next, &mut *app);
                next
            };
            let next_editor = std::mem::take(editor);
            (
                next,
                next_editor,
                None,
                old_finished.clone(),
                old_canonical.clone(),
            )
        } else {
            match cached_target {
                Some(cached) if live => (
                    cached.app,
                    cached.editor,
                    cached.association,
                    cached.finished,
                    cached.canonical,
                ),
                Some(mut cached) => {
                    let mut next = App::restore(&replay);
                    merge_view(&mut next, &mut cached.app);
                    (
                        next,
                        cached.editor,
                        cached.association,
                        cached.finished,
                        cached.canonical,
                    )
                }
                None => (App::restore(&replay), Editor::new(), None, None, None),
            }
        };
        if !same_current {
            next_editor.reset_history();
            next.text_selection.clear();
        }
        next.working_directory = Some(metadata.working_directory.clone());
        next.session = Some(metadata);
        next.apply_event(context);
        next.theme = current_theme;
        next.normalize_selection();
        next.normalize_anchor();
        // Only the active stream may resurrect a running projection. A cached
        // association for a finished task must not make stream None run again.
        let association = stream;
        next.running = association.is_some() || state.active_task.is_some();
        next.cancelling = state
            .active_task
            .as_ref()
            .is_some_and(|task| task.cancel_requested);
        self.association = association;
        self.finished = if self.association.is_some() {
            None
        } else {
            selected_finished
        };
        self.canonical = if self.association.is_some() {
            None
        } else {
            selected_canonical
        };
        crate::frontend::tui::transcript_search::cancel(app);
        crate::frontend::tui::path_completion::cancel(app);
        if same_current {
            *app = next;
            *editor = next_editor;
        } else {
            let previous = std::mem::replace(app, next);
            let draft = std::mem::replace(editor, next_editor);
            if let Some(metadata) = &previous.session {
                self.cached.insert(
                    metadata.id.clone(),
                    CachedSession {
                        app: previous,
                        editor: draft,
                        association: old_association,
                        finished: old_finished,
                        canonical: old_canonical,
                    },
                );
            }
        }
        self.picker = None;
        self.pending = false;
    }

    fn history_restored(
        &mut self,
        app: &mut App,
        metadata: crate::client::SessionMetadata,
        replay: crate::client::history::History,
        context: AgentEvent,
        _state: crate::client::SessionState,
        association: Option<Association>,
    ) {
        let target = metadata.id.clone();
        let Some(association) = association else {
            return;
        };
        // Canonical repair of the associated view only; never select it and
        // never mark the task consumed before its Finished packet arrives.
        if app
            .session
            .as_ref()
            .is_some_and(|current| current.id == target)
        {
            if !self
                .association
                .as_ref()
                .is_some_and(|current| same_association(current, &association))
            {
                return;
            }
            let mut repaired = App::restore(&replay);
            merge_view(&mut repaired, &mut *app);
            repaired.working_directory = Some(metadata.working_directory.clone());
            repaired.session = Some(metadata);
            repaired.apply_event(context);
            repaired.theme = app.theme;
            repaired.text_selection.clear();
            repaired.normalize_selection();
            repaired.normalize_anchor();
            *app = repaired;
            self.canonical = Some(association.task_id);
        } else if let Some(cached) = self.cached.get_mut(&target) {
            if !cached
                .association
                .as_ref()
                .is_some_and(|current| same_association(current, &association))
            {
                return;
            }
            let mut repaired = App::restore(&replay);
            merge_view(&mut repaired, &mut cached.app);
            repaired.working_directory = Some(metadata.working_directory.clone());
            repaired.session = Some(metadata);
            repaired.apply_event(context);
            repaired.theme = cached.app.theme;
            repaired.text_selection.clear();
            repaired.normalize_selection();
            repaired.normalize_anchor();
            cached.app = repaired;
            cached.canonical = Some(association.task_id);
        }
    }

    fn metadata_update(
        &mut self,
        app: &mut App,
        metadata: crate::client::SessionMetadata,
        renamed: bool,
    ) {
        if app
            .session
            .as_ref()
            .is_some_and(|active| active.id == metadata.id)
        {
            app.session = Some(metadata.clone());
        }
        if let Some(cached) = self.cached.get_mut(&metadata.id) {
            cached.app.session = Some(metadata.clone());
        }
        if let Some(picker) = &mut self.picker {
            let preferred = picker
                .rename
                .as_ref()
                .map(|rename| rename.id.clone())
                .or_else(|| {
                    picker
                        .selected_session()
                        .map(|session| session.metadata.id.clone())
                });
            if let Some(session) = picker
                .sessions
                .iter_mut()
                .find(|session| session.metadata.id == metadata.id)
            {
                session.metadata = metadata;
            }
            if renamed {
                picker.rename = None;
            }
            picker.refilter(preferred.as_deref());
        }
    }

    fn agent(
        &mut self,
        app: &mut App,
        session_id: String,
        event: AgentEvent,
        association: Option<Association>,
    ) {
        // Finished owns the single terminal; Agent Turn* duplicates are ignored.
        if matches!(
            event,
            AgentEvent::TurnCompleted | AgentEvent::TurnInterrupted | AgentEvent::TurnFailed { .. }
        ) {
            return;
        }
        let target = association
            .as_ref()
            .map_or(session_id.as_str(), |association| {
                association.session_id.as_str()
            });
        let task_id = association
            .as_ref()
            .map(|association| association.task_id.clone());
        if app
            .session
            .as_ref()
            .is_some_and(|active| active.id == target)
        {
            if let Some(task_id) = &task_id {
                if self.finished.as_deref() == Some(task_id.as_str()) {
                    return;
                }
                if self.association.as_ref().is_some_and(|current| {
                    !same_association(current, association.as_ref().unwrap())
                }) {
                    return;
                }
                self.association = association;
            }
            app.apply_event(event);
            app.running = true;
        } else if let Some(cached) = self.cached.get_mut(target) {
            if let Some(task_id) = &task_id {
                if cached.finished.as_deref() == Some(task_id.as_str()) {
                    return;
                }
                if cached.association.as_ref().is_some_and(|current| {
                    !same_association(current, association.as_ref().unwrap())
                }) {
                    return;
                }
                cached.association = association;
            }
            cached.app.apply_event(event);
            cached.app.running = true;
        }
    }

    fn finished(
        &mut self,
        app: &mut App,
        finished: crate::client::TaskFinished,
        association: Option<Association>,
    ) {
        let target = association
            .as_ref()
            .map(|association| association.session_id.as_str())
            .unwrap_or_default();
        let task_id = association
            .as_ref()
            .map(|association| association.task_id.clone());
        if app
            .session
            .as_ref()
            .is_some_and(|active| active.id == target)
        {
            finish_app(
                app,
                &mut self.association,
                &mut self.finished,
                &mut self.canonical,
                association.as_ref(),
                task_id.as_deref(),
                &finished,
            );
        } else if let Some(cached) = self.cached.get_mut(target) {
            finish_app(
                &mut cached.app,
                &mut cached.association,
                &mut cached.finished,
                &mut cached.canonical,
                association.as_ref(),
                task_id.as_deref(),
                &finished,
            );
        }
    }
}

fn merge_view(next: &mut App, previous: &mut App) {
    next.follow = previous.follow;
    next.anchor = previous
        .anchor
        .filter(|anchor| anchor.record < next.records.len());
    next.selected = previous.selected.min(next.records.len().saturating_sub(1));
    next.focus = previous.focus;
    next.history = std::mem::take(&mut previous.history);
    for (record, old) in next.records.iter_mut().zip(&previous.records) {
        match (record, old) {
            (
                Record::Reasoning { text, expanded, .. },
                Record::Reasoning {
                    text: old_text,
                    expanded: old_expanded,
                    ..
                },
            ) if text == old_text => *expanded = *old_expanded,
            (
                Record::Tool {
                    call_id, expanded, ..
                },
                Record::Tool {
                    call_id: old_id,
                    expanded: old_expanded,
                    ..
                },
            ) if call_id == old_id => *expanded = *old_expanded,
            (
                Record::Activity {
                    expanded, manual, ..
                },
                Record::Activity {
                    expanded: old_expanded,
                    manual: old_manual,
                    ..
                },
            ) => {
                *expanded = *old_expanded;
                *manual = *old_manual;
            }
            _ => {}
        }
    }
}

fn same_association(left: &Association, right: &Association) -> bool {
    left.session_id == right.session_id
        && left.task_id == right.task_id
        && left.message_id == right.message_id
}

fn finish_app(
    app: &mut App,
    association_slot: &mut Option<Association>,
    finished_slot: &mut Option<String>,
    canonical_slot: &mut Option<String>,
    incoming: Option<&Association>,
    task_id: Option<&str>,
    finished: &crate::client::TaskFinished,
) {
    let Some(incoming) = incoming else {
        return;
    };
    match association_slot.as_ref() {
        Some(observed) if same_association(observed, incoming) => {}
        _ => return,
    }
    if let Some(task_id) = task_id
        && finished_slot.as_deref() == Some(task_id)
    {
        return;
    }
    let canonical = task_id.is_some_and(|task_id| canonical_slot.as_deref() == Some(task_id));
    if !canonical {
        let event = match finished.status.as_str() {
            "completed" => AgentEvent::TurnCompleted,
            "interrupted" => AgentEvent::TurnInterrupted,
            _ => AgentEvent::TurnFailed {
                message: finished
                    .error
                    .as_ref()
                    .map_or_else(|| "Task failed.".to_owned(), |error| error.message.clone()),
            },
        };
        app.apply_event(event);
    }
    app.running = false;
    app.cancelling = false;
    if let Some(task_id) = task_id {
        *finished_slot = Some(task_id.to_owned());
    }
    canonical_slot.take();
    association_slot.take();
    let mut notes = Vec::new();
    if let Some(error) = &finished.error {
        notes.push(format!("{}: {}", error.code, error.message));
    }
    if let Some(persistence) = &finished.persistence {
        notes.push(format!("persistence: {persistence}"));
    }
    if finished.history_required && !canonical {
        notes.push("history refresh required".to_owned());
    }
    if !notes.is_empty() {
        app.notify(format!("Task finished ({})", notes.join("; ")));
    }
}

impl Sessions {
    /// Returns whether the picker consumed the key and an optional core request.
    pub fn key(&mut self, app: &mut App, key: KeyEvent) -> (bool, Option<SessionCommand>) {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return (self.picker.is_some() || self.pending, None);
        }
        if self.pending {
            return (true, None);
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('s' | 'S'))
        {
            if self.picker.take().is_some() {
                return (true, None);
            }
            self.pending = true;
            return (true, Some(SessionCommand::List));
        }
        let Some(picker) = &mut self.picker else {
            return (false, None);
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);

        // Rename editing owns printable keys; n/r are title text here.
        if let Some(rename) = &mut picker.rename {
            match key.code {
                KeyCode::Esc => picker.rename = None,
                KeyCode::Backspace => {
                    rename.title.pop();
                }
                KeyCode::Char('c') if ctrl => picker.rename = None,
                KeyCode::Char(c) if !ctrl && !alt => rename.title.push(c),
                KeyCode::Enter if !rename.title.trim().is_empty() => {
                    self.pending = true;
                    return (
                        true,
                        Some(SessionCommand::Rename {
                            session_id: rename.id.clone(),
                            title: rename.title.clone(),
                        }),
                    );
                }
                _ => {}
            }
            return (true, None);
        }

        // Search text editing; Up/Down and Enter continue to the shared guards.
        if picker.query.is_some() {
            match key.code {
                KeyCode::Esc => {
                    picker.query = None;
                    picker.refresh();
                }
                KeyCode::Char('c') if ctrl => {
                    picker.query = None;
                    picker.refresh();
                }
                KeyCode::Backspace => {
                    if let Some(query) = picker.query.as_mut() {
                        pop_last_grapheme(query);
                    }
                    picker.refresh();
                }
                KeyCode::Char(c) if !ctrl && !alt => {
                    if let Some(query) = picker.query.as_mut() {
                        query.push(c);
                    }
                    picker.refresh();
                }
                _ => {}
            }
            if !matches!(key.code, KeyCode::Up | KeyCode::Down | KeyCode::Enter) {
                return (true, None);
            }
        }

        match key.code {
            KeyCode::Esc => self.picker = None,
            KeyCode::Char('c') if ctrl => self.picker = None,
            KeyCode::Up => picker.selected = picker.selected.saturating_sub(1),
            KeyCode::Down => {
                picker.selected = (picker.selected + 1).min(picker.visible.len().saturating_sub(1))
            }
            KeyCode::Char('/') => {
                picker.query = Some(String::new());
                picker.refresh();
            }
            KeyCode::Char('r') => {
                if let Some(session) = picker
                    .selected_session()
                    .filter(|session| session.unavailable.is_none())
                {
                    picker.rename = Some(Rename {
                        id: session.metadata.id.clone(),
                        title: session.metadata.title.clone(),
                    });
                }
            }
            KeyCode::Enter | KeyCode::Char('n') => {
                let command = if key.code == KeyCode::Char('n') {
                    SessionCommand::Create
                } else {
                    let Some(session) = picker.selected_session() else {
                        return (true, None);
                    };
                    if app
                        .session
                        .as_ref()
                        .is_some_and(|active| active.id == session.metadata.id)
                    {
                        self.picker = None;
                        return (true, None);
                    }
                    if let Some(error) = &session.unavailable {
                        app.notify(error.clone());
                        return (true, None);
                    }
                    if session.in_use {
                        app.notify("This session is in use by another process.".to_owned());
                        return (true, None);
                    }
                    SessionCommand::Open {
                        session_id: session.metadata.id.clone(),
                    }
                };
                self.pending = true;
                return (true, Some(command));
            }
            _ => {}
        }
        (true, None)
    }

    pub fn paste(&mut self, text: &str) {
        if self.pending {
            return;
        }
        let Some(picker) = self.picker.as_mut() else {
            return;
        };
        if let Some(rename) = picker.rename.as_mut() {
            rename.title.push_str(&text.replace(['\r', '\n'], " "));
            return;
        }
        if picker.query.is_some() {
            let text = text.replace(['\r', '\n'], " ");
            if let Some(query) = picker.query.as_mut() {
                query.push_str(&text);
            }
            picker.refresh();
        }
    }

    pub fn draw(&self, frame: &mut Frame, theme: theme::Theme) {
        let Some(picker) = &self.picker else {
            return;
        };
        let screen = frame.area();
        let width = screen.width.saturating_sub(4).min(90);
        let height = screen.height.saturating_sub(4).min(18);
        if width < 4 || height < 4 {
            return;
        }
        let area = Rect::new(
            screen.x + (screen.width - width) / 2,
            screen.y + (screen.height - height) / 2,
            width,
            height,
        );
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Sessions ")
            .border_style(Style::default().fg(theme.accent));
        let inner = block.inner(area);
        frame.render_widget(Clear, area);
        frame.render_widget(block, area);
        let mut lines: Vec<Line> = Vec::new();
        if let Some(query) = &picker.query {
            // Show the longest fitting query suffix so the caret stays visible
            // when the query is wider than the picker.
            let available = (inner.width as usize).saturating_sub("Search: ".len() + 1);
            let shown = textwrap::visible_suffix(query, available.max(1));
            lines.push(Line::from(vec![
                Span::styled("Search: ", Style::default().fg(theme.accent)),
                Span::styled(shown, Style::default().fg(theme.secondary)),
                Span::styled("▌", Style::default().fg(theme.accent)),
            ]));
        }
        // Reserve the blank separator and footer for the hint line.
        let available = (inner.height as usize)
            .saturating_sub(lines.len())
            .saturating_sub(2);
        if picker.visible.is_empty() {
            lines.push(Line::from(Span::styled(
                if picker.query.is_some() {
                    "No sessions match the search.".to_owned()
                } else {
                    "No saved sessions.".to_owned()
                },
                Style::default().fg(theme.secondary),
            )));
        } else {
            let count = (available / 2).max(1);
            let start = picker.selected.saturating_sub(count.saturating_sub(1));
            for (index, session_index) in picker.visible.iter().enumerate().skip(start).take(count)
            {
                let session = &picker.sessions[*session_index];
                let marker = if index == picker.selected {
                    "› "
                } else {
                    "  "
                };
                let state = if session.unavailable.is_some() {
                    " · unavailable"
                } else if picker.active_id.as_ref() == Some(&session.metadata.id) {
                    " · current"
                } else if session.in_use {
                    " · in use"
                } else if session.status == TaskStatus::Interrupted {
                    " · interrupted"
                } else {
                    ""
                };
                let seconds = crate::client::history::now_ms()
                    .unwrap_or(session.updated_at)
                    .saturating_sub(session.updated_at)
                    / 1000;
                let age = if seconds < 60 {
                    "now".to_owned()
                } else if seconds < 3600 {
                    format!("{}m ago", seconds / 60)
                } else if seconds < 86400 {
                    format!("{}h ago", seconds / 3600)
                } else {
                    format!("{}d ago", seconds / 86400)
                };
                let text = format!("{marker}{}{state} · {age}", session.metadata.title);
                let style = if index == picker.selected {
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.secondary)
                };
                lines.push(Line::from(Span::styled(text, style)));
                lines.push(Line::from(Span::styled(
                    format!(
                        "  {} · {}",
                        session.metadata.model,
                        session.metadata.working_directory.display()
                    ),
                    Style::default().fg(theme.secondary),
                )));
            }
        }
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            if self.pending {
                "Loading…".to_owned()
            } else if let Some(rename) = &picker.rename {
                format!("Rename: {}  · Enter save · Esc cancel", rename.title)
            } else if picker.query.is_some() {
                if inner.width >= 50 {
                    "Type to filter · Up/Down select · Enter open · Esc clear".to_owned()
                } else {
                    "Type · Up/Down · Enter · Esc".to_owned()
                }
            } else if inner.width >= 54 {
                "Enter open · n new · r rename · / search · Esc close".to_owned()
            } else {
                "Enter open · / search".to_owned()
            },
            Style::default().fg(theme.secondary),
        )));
        frame.render_widget(Paragraph::new(lines), inner);
    }
}
