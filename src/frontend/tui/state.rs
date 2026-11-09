use std::collections::HashMap;

use ratatui::{layout::Rect, text::Line};
use serde_json::Value;
use std::time::{Duration, Instant};

use crate::client::AgentEvent;
use crate::frontend::tui::history::InputHistory;
use crate::frontend::tui::selection::{TextSelection, ViewportRow, VisualAnchor};
use crate::frontend::tui::theme::Theme;
use crate::input::{ImageAttachment, UserInput};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    Input,
    Transcript,
}

pub struct ContextInfo {
    pub model: String,
    pub tokens: Option<usize>,
    pub window: Option<usize>,
    pub input_limit: Option<usize>,
}

/// Reading position expressed as a stable record index plus an original-text
/// byte offset into that record. Records are append-only, so the index stays
/// valid across resizes, tool expansion and new events.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Anchor {
    pub record: usize,
    pub offset: usize,
}

/// One wrapped visual line plus the original-text byte offset where its body
/// starts. The source offset keeps the reading anchor stable across resize and
/// reflow.
#[derive(Clone)]
pub struct VisualLine {
    pub line: Line<'static>,
    pub source_start: usize,
}

/// Cached layout for one record. It is invalidated by width, expansion or
/// tool-result changes; the selection marker is overlaid at render time so the
/// cache key does not need focus or selection.
pub struct CachedLayout {
    pub width: usize,
    pub expanded: bool,
    pub has_output: bool,
    pub lines: Vec<VisualLine>,
}

/// An independently paged log sub-view for a tool that spilled output to disk.
pub struct LogView {
    pub path: String,
    pub offset: u64,
    pub next: u64,
    pub eof: bool,
    pub text: String,
    /// In-page display-line scroll offset.
    pub scroll: usize,
    pub history: Vec<u64>,
    pub saved_follow: bool,
    pub saved_anchor: Option<Anchor>,
    pub saved_visual_anchor: Option<VisualAnchor>,
}

/// One visible entry in the work record.
#[derive(Debug)]
pub enum Record {
    User {
        text: String,
        images: Vec<ImageAttachment>,
    },
    Assistant {
        text: String,
    },
    Reasoning {
        text: String,
        expanded: bool,
        auto_collapse: bool,
    },
    Tool {
        call_id: String,
        name: String,
        arguments: String,
        /// Raw tool result; `None` while the call is still running.
        output: Option<Value>,
        expanded: bool,
    },
    /// Frontend-only disclosure for the process of one user task. It is created
    /// at task start, right after the user record, so raw record order is kept.
    /// Members are the following records up to `end`; trailing answer records
    /// stay outside. `end == None` means the task is still running.
    Activity {
        end: Option<usize>,
        expanded: bool,
        failed: bool,
        interrupted: bool,
        /// Set when the reader toggled this header; auto-collapse never
        /// overrides an explicit choice.
        manual: bool,
    },
    Compaction {
        tokens_before: usize,
        tokens_after: usize,
    },
    Error {
        message: String,
    },
}

/// TUI application state. It owns only display data and interaction state; it
/// never holds the agent, conversation or tools.
pub struct App {
    pub records: Vec<Record>,
    pub session: Option<crate::client::SessionMetadata>,
    pub session_button_area: Rect,
    pub layouts: Vec<Option<CachedLayout>>,
    response_records: HashMap<String, usize>,
    pub running: bool,
    /// Esc cancellation was requested and its terminal event is still pending.
    pub cancelling: bool,
    pub follow: bool,
    pub anchor: Option<Anchor>,
    pub focus: Focus,
    pub selected: usize,
    pub status: Option<String>,
    pub feedback: Option<(String, Instant)>,
    pub help_open: bool,
    pub help_scroll: usize,
    pub help_area: Rect,
    pub help_button_area: Rect,
    pub latest_button_area: Rect,
    pub unread_output: bool,
    pub usage: Option<(Option<u64>, Option<u64>)>,
    pub context: Option<ContextInfo>,
    pub working_directory: Option<std::path::PathBuf>,
    pub text_selection: TextSelection,
    /// Rendered transcript rows from the last draw: logical row key plus its
    /// screen row and cells, used for cross-viewport selection.
    pub transcript_rows: Vec<ViewportRow>,
    /// Exact top visual row at a known width, preferred over source-offset
    /// resolution while the width is unchanged.
    pub visual_anchor: Option<VisualAnchor>,
    pub log_view: Option<LogView>,
    pub quit_after_turn: bool,
    /// Active palette for every TUI surface. The front end starts from
    /// `SAKI_THEME` (auto native fallback by default) and may update it through
    /// `set_theme` when input-safe detection reports a change.
    pub theme: Theme,
    /// Per-process input history (submitted + Ctrl+C-cleared drafts).
    pub history: InputHistory,
    /// Active rendered-transcript search, or `None` when closed.
    pub transcript_search: Option<crate::frontend::tui::transcript_search::TranscriptSearch>,
    /// Active path-completion popup, or `None` when closed.
    pub path_completion: Option<crate::frontend::tui::path_completion::PathCompletion>,
    /// Monotonic record/projection revision; bumped whenever rendered content,
    /// expansion or projection changes so search never trusts a rebuilt cache.
    pub transcript_revision: u64,
    /// Last drawn input content width, used by visual line navigation.
    pub input_width: usize,
    /// Last drawn transcript size, used for paging math.
    pub last_width: usize,
    pub last_height: usize,
    /// Screen area of the transcript/log pane, used for mouse hit testing.
    pub transcript_area: Rect,
    pub input_area: Rect,
    pub disclosure_buttons: Vec<(Rect, usize)>,
    pub disclosure_press: Option<usize>,
    /// Index of the open Activity marker for the task in progress.
    task_marker: Option<usize>,
    /// First record index belonging to the current task's display projection.
    task_record_start: Option<usize>,
    /// Record indices of every Activity marker, in creation order.
    activity_markers: Vec<usize>,
}

impl App {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            session: None,
            session_button_area: Rect::default(),
            layouts: Vec::new(),
            response_records: HashMap::new(),
            running: false,
            cancelling: false,
            follow: true,
            anchor: None,
            focus: Focus::Input,
            selected: 0,
            status: None,
            feedback: None,
            help_open: false,
            help_scroll: 0,
            help_area: Rect::default(),
            help_button_area: Rect::default(),
            latest_button_area: Rect::default(),
            unread_output: false,
            usage: None,
            context: None,
            working_directory: None,
            text_selection: TextSelection::default(),
            transcript_rows: Vec::new(),
            visual_anchor: None,
            log_view: None,
            quit_after_turn: false,
            theme: Theme::fallback(),
            history: InputHistory::new(),
            transcript_search: None,
            path_completion: None,
            transcript_revision: 0,
            input_width: 80,
            last_width: 80,
            last_height: 24,
            transcript_area: Rect::new(0, 0, 0, 0),
            input_area: Rect::default(),
            disclosure_buttons: Vec::new(),
            disclosure_press: None,
            task_marker: None,
            task_record_start: None,
            activity_markers: Vec::new(),
        }
    }

    pub fn restore(replay: &crate::client::history::History) -> Self {
        use crate::{
            client::history::{HistoryEntry, TaskStatus},
            client::types::{InputContentPart, InputItem, InputMessageContent, MessageRole},
        };
        let mut app = Self::new();
        let mut pending_task = false;
        let mut marker: Option<usize> = None;
        for record in &replay.history {
            match record {
                HistoryEntry::Messages { items } => {
                    for item in items {
                        match item {
                            InputItem::Message {
                                role: MessageRole::User,
                                content,
                            } => {
                                let mut message = UserInput::default();
                                match content {
                                    InputMessageContent::Text(text) => message.text = text.clone(),
                                    InputMessageContent::Parts(parts) => {
                                        for part in parts {
                                            match part {
                                                InputContentPart::Text { text } => {
                                                    message.text.push_str(text)
                                                }
                                                InputContentPart::Image { url } => {
                                                    message.images.push(ImageAttachment::new(
                                                        format!(
                                                            "image #{}",
                                                            message.images.len() + 1
                                                        ),
                                                        url.clone(),
                                                    ))
                                                }
                                            }
                                        }
                                    }
                                }
                                app.history.push(message.clone());
                                app.push(Record::User {
                                    text: message.text,
                                    images: message.images,
                                });
                                if pending_task {
                                    marker = Some(app.records.len());
                                    app.push_open_activity();
                                    pending_task = false;
                                }
                            }
                            InputItem::Message {
                                role: MessageRole::Assistant,
                                content,
                            } => {
                                let text = match content {
                                    InputMessageContent::Text(text) => text.clone(),
                                    InputMessageContent::Parts(parts) => parts
                                        .iter()
                                        .filter_map(|part| match part {
                                            InputContentPart::Text { text } => Some(text.as_str()),
                                            _ => None,
                                        })
                                        .collect::<String>(),
                                };
                                app.push(Record::Assistant { text });
                            }
                            InputItem::Reasoning { content, .. } => {
                                let text: String =
                                    content.iter().map(|part| part.text.as_str()).collect();
                                if !text.is_empty() {
                                    app.push(Record::Reasoning {
                                        text,
                                        expanded: false,
                                        auto_collapse: false,
                                    });
                                }
                            }
                            InputItem::ToolCall {
                                call_id,
                                name,
                                arguments,
                            } => app.apply_event(AgentEvent::ToolStarted {
                                call_id: call_id.clone(),
                                name: name.clone(),
                                arguments: arguments.clone(),
                            }),
                            InputItem::ToolResult { call_id, output } => {
                                app.apply_event(AgentEvent::ToolFinished {
                                    call_id: call_id.clone(),
                                    name: String::new(),
                                    output: serde_json::from_str(output).unwrap_or_else(|_| {
                                        serde_json::Value::String(output.clone())
                                    }),
                                })
                            }
                            _ => {}
                        }
                    }
                }
                HistoryEntry::Compaction {
                    tokens_before,
                    tokens_after,
                    ..
                } => app.push(Record::Compaction {
                    tokens_before: *tokens_before,
                    tokens_after: *tokens_after,
                }),
                HistoryEntry::TaskStarted { .. } => {
                    pending_task = true;
                    marker = None;
                }
                HistoryEntry::TaskFinished { status, error, .. } => {
                    if let Some(index) = marker.take() {
                        let end = app.task_boundary(index);
                        let completed = matches!(status, TaskStatus::Completed);
                        if let Record::Activity {
                            end: slot,
                            expanded,
                            failed,
                            interrupted,
                            ..
                        } = &mut app.records[index]
                        {
                            *slot = Some(end);
                            *failed = !completed;
                            *interrupted = matches!(status, TaskStatus::Interrupted);
                            // Completed history starts collapsed; a failed or
                            // interrupted task keeps its process visible.
                            *expanded = !completed;
                        }
                    }
                    if let Some(message) = error {
                        app.push(Record::Error {
                            message: message.clone(),
                        });
                    }
                }
            }
        }
        // A restored task with no completion record must not stay open: close
        // its range at the replay boundary so records appended after reopening
        // belong to a new task, never to the interrupted one. The boundary is
        // taken before the synthetic status error so that record stays visible
        // outside the group.
        if let Some(index) = marker
            && matches!(
                app.records.get(index),
                Some(Record::Activity { end: None, .. })
            )
        {
            let end = app.records.len();
            if let Record::Activity {
                end: slot,
                expanded,
                failed,
                interrupted,
                ..
            } = &mut app.records[index]
            {
                *slot = Some(end);
                *failed = true;
                *interrupted = true;
                *expanded = true;
            }
        }
        if matches!(replay.status, TaskStatus::Interrupted | TaskStatus::Unknown)
            && replay.has_task_history()
        {
            app.push(Record::Error { message: format!("Previous task: {:?}. No tools were re-executed. Check current state before continuing.", replay.status) });
        }
        app
    }

    pub fn push(&mut self, record: Record) {
        if (!self.follow || self.log_view.is_some())
            && !matches!(record, Record::User { .. } | Record::Activity { .. })
        {
            self.unread_output = true;
        }
        if matches!(record, Record::Activity { .. }) {
            self.activity_markers.push(self.records.len());
        }
        self.records.push(record);
        self.layouts.push(None);
        self.bump_transcript_revision();
    }

    fn bump_transcript_revision(&mut self) {
        self.transcript_revision = self.transcript_revision.wrapping_add(1);
    }

    /// One boundary for record-content invalidation: layout cache, selection
    /// mapping and the search revision always move together.
    fn invalidate_record(&mut self, index: usize) {
        if index < self.layouts.len() {
            self.layouts[index] = None;
        }
        self.text_selection.invalidate_record(index);
        self.bump_transcript_revision();
    }

    fn push_open_activity(&mut self) {
        self.push(Record::Activity {
            end: None,
            expanded: true,
            failed: false,
            interrupted: false,
            manual: false,
        });
    }

    /// Records the user message. It deliberately does not resume following, so
    /// a reader who scrolled up keeps their position until they press End.
    /// The Activity marker is created here, at task start, so raw record order
    /// is retained and every later record keeps its identity.
    pub fn submit(&mut self, message: impl Into<UserInput>) {
        let message = message.into();
        self.response_records.clear();
        self.task_record_start = Some(self.records.len());
        self.history.push(message.clone());
        self.push(Record::User {
            text: message.text,
            images: message.images,
        });
        self.task_marker = Some(self.records.len());
        self.push_open_activity();
        self.running = true;
        self.status = None;
    }

    pub fn apply_event(&mut self, event: AgentEvent) {
        let reasoning = matches!(
            &event,
            AgentEvent::Reasoning { .. } | AgentEvent::ReasoningDelta { .. }
        );
        match event {
            AgentEvent::TaskStarted { .. } | AgentEvent::ModelRequestStarted { .. } => {}
            AgentEvent::ContextUpdated {
                model,
                context_tokens,
                context_window,
                input_limit,
            } => {
                self.context = Some(ContextInfo {
                    model,
                    tokens: context_tokens,
                    window: context_window,
                    input_limit,
                });
            }
            AgentEvent::ResponseTextDelta { item_id, delta }
            | AgentEvent::ReasoningDelta { item_id, delta } => {
                if delta.is_empty() {
                    return;
                }
                if !self.follow || self.log_view.is_some() {
                    self.unread_output = true;
                }
                if let Some(&index) = self.response_records.get(&item_id) {
                    if let Record::Assistant { text } | Record::Reasoning { text, .. } =
                        &mut self.records[index]
                    {
                        text.push_str(&delta);
                    }
                    self.invalidate_record(index);
                } else {
                    let index = self.records.len();
                    self.push(if reasoning {
                        Record::Reasoning {
                            text: delta,
                            expanded: true,
                            auto_collapse: true,
                        }
                    } else {
                        Record::Assistant { text: delta }
                    });
                    self.response_records.insert(item_id, index);
                }
            }
            AgentEvent::ResponseText { item_id, text }
            | AgentEvent::Reasoning { item_id, text } => {
                if let Some(&index) = self.response_records.get(&item_id) {
                    match &mut self.records[index] {
                        Record::Assistant { text: stored }
                        | Record::Reasoning { text: stored, .. } => {
                            // Encrypted-only final reasoning must not erase streamed text.
                            if !reasoning || !text.is_empty() {
                                *stored = text;
                                self.invalidate_record(index);
                            }
                        }
                        _ => unreachable!(),
                    }
                } else {
                    if reasoning && text.is_empty() {
                        return;
                    }
                    let record = if reasoning {
                        Record::Reasoning {
                            text,
                            expanded: true,
                            auto_collapse: true,
                        }
                    } else {
                        Record::Assistant { text }
                    };
                    let index = self.records.len();
                    self.push(record);
                    self.response_records.insert(item_id.clone(), index);
                }
                if reasoning {
                    self.complete_reasoning(&item_id);
                }
            }
            AgentEvent::ReasoningCompleted { item_id } => self.complete_reasoning(&item_id),
            AgentEvent::ToolStarted {
                call_id,
                name,
                arguments,
            } => self.push(Record::Tool {
                call_id,
                name,
                arguments,
                output: None,
                expanded: false,
            }),
            AgentEvent::ToolFinished {
                call_id, output, ..
            } => {
                if !self.follow || self.log_view.is_some() {
                    self.unread_output = true;
                }
                let start = self.task_record_start.unwrap_or(0);
                let existing = self.records.iter().enumerate().rev().find_map(|(index, record)| {
                    (index >= start
                        && matches!(record, Record::Tool { call_id: id, output: None, .. } if *id == call_id))
                        .then_some(index)
                });
                match existing {
                    // Prefer the most recent unfinished record with this
                    // call_id and never attach a result to a different one.
                    Some(index) => {
                        if let Record::Tool { output: slot, .. } = &mut self.records[index] {
                            *slot = Some(output);
                        }
                        self.invalidate_record(index);
                    }
                    None => self.push(Record::Tool {
                        call_id,
                        name: String::new(),
                        arguments: String::new(),
                        output: Some(output),
                        expanded: false,
                    }),
                }
            }
            AgentEvent::Compaction {
                tokens_before,
                tokens_after,
                ..
            } => self.push(Record::Compaction {
                tokens_before,
                tokens_after,
            }),
            AgentEvent::Usage {
                input_tokens,
                output_tokens,
            } => {
                self.response_records.clear();
                if input_tokens.is_some() || output_tokens.is_some() {
                    // Keep missing fields as `None` so the UI shows `?` instead
                    // of a fake zero.
                    self.usage = Some((input_tokens, output_tokens));
                }
            }
            AgentEvent::TurnCompleted => self.finish_turn(true),
            AgentEvent::TurnInterrupted => self.finish_turn_interrupted(),
            AgentEvent::TurnFailed { message } => {
                self.push(Record::Error { message });
                self.finish_turn(false);
            }
        }
    }

    fn complete_reasoning(&mut self, item_id: &str) {
        let Some(&index) = self.response_records.get(item_id) else {
            return;
        };
        let reading = !self.follow
            || self.log_view.is_some()
            || (self.focus == Focus::Transcript && self.selected == index)
            || self.text_selection.text().is_some();
        if let Record::Reasoning {
            expanded,
            auto_collapse,
            ..
        } = &mut self.records[index]
            && *auto_collapse
        {
            if !reading {
                *expanded = false;
                self.text_selection.invalidate_record(index);
            }
            // Completion and user toggles consume the automatic transition once.
            *auto_collapse = false;
            self.layouts[index] = None;
        }
        self.bump_transcript_revision();
    }

    fn finish_turn(&mut self, completed: bool) {
        if let Some(marker) = self.task_marker.take() {
            let end = self.task_boundary(marker);
            let manual = self.activity_manual(marker, end);
            let collapse = completed && self.follow && !self.reading(marker, end);
            // Collapsing hides member rows, so a logical selection that may
            // cover them is cancelled rather than silently remapped.
            if completed && collapse && self.text_selection.active() {
                self.text_selection.clear();
            }
            if let Record::Activity {
                end: slot,
                expanded,
                failed,
                ..
            } = &mut self.records[marker]
            {
                *slot = Some(end);
                *failed = !completed;
                // A manual choice is never overridden. A failed task keeps its
                // process visible unless the reader already collapsed it; the
                // Error record and status badge still identify the failure.
                if !manual {
                    *expanded = if completed { !collapse } else { true };
                }
            }
            // The answer separator depends on the finalized boundary, so any
            // Assistant layout cached while streaming must be rebuilt; the
            // cache key keeps `has_output` meaning a real tool result only.
            for index in marker + 1..self.records.len() {
                if matches!(self.records[index], Record::Assistant { .. }) {
                    self.layouts[index] = None;
                }
            }
            self.bump_transcript_revision();
        }
        self.response_records.clear();
        self.task_record_start = None;
        self.running = false;
        self.cancelling = false;
        self.status = None;
    }

    fn finish_turn_interrupted(&mut self) {
        if let Some(marker) = self.task_marker.take() {
            let end = self.task_boundary(marker);
            if let Record::Activity {
                end: slot,
                expanded,
                failed,
                interrupted,
                ..
            } = &mut self.records[marker]
            {
                *slot = Some(end);
                *failed = false;
                *interrupted = true;
                *expanded = true;
            }
            self.layouts[marker] = None;
            self.bump_transcript_revision();
        }
        self.push(Record::Error {
            message: "Interrupted.".to_owned(),
        });
        self.response_records.clear();
        self.task_record_start = None;
        self.running = false;
        self.cancelling = false;
        self.status = None;
    }

    /// Whether the reader explicitly toggled the header or any member detail.
    fn activity_manual(&self, marker: usize, end: usize) -> bool {
        matches!(
            self.records.get(marker),
            Some(Record::Activity { manual: true, .. })
        ) || (marker + 1..end).any(|index| match &self.records[index] {
            Record::Reasoning {
                expanded,
                auto_collapse,
                ..
            } => *expanded && !*auto_collapse,
            Record::Tool { expanded, .. } => *expanded,
            _ => false,
        })
    }

    /// A reader is looking at history when not following, selecting, or inside
    /// a log/selection; their position must never be collapsed away.
    fn reading(&self, marker: usize, end: usize) -> bool {
        !self.follow
            || self.log_view.is_some()
            || self.text_selection.active()
            || (self.focus == Focus::Transcript && self.selected >= marker && self.selected < end)
    }

    /// One past the last reasoning or tool record of the task that starts at
    /// `marker`. Trailing answer records stay outside the process.
    fn task_boundary(&self, marker: usize) -> usize {
        (marker + 1..self.records.len())
            .rev()
            .find(|index| {
                matches!(
                    self.records[*index],
                    Record::Reasoning { .. } | Record::Tool { .. }
                )
            })
            .map_or(marker + 1, |index| index + 1)
    }

    /// Members of one Activity marker as a record range.
    pub fn activity_members(&self, marker: usize) -> std::ops::Range<usize> {
        let Some(Record::Activity { end, .. }) = self.records.get(marker) else {
            return 0..0;
        };
        let end = end.unwrap_or(self.records.len()).min(self.records.len());
        marker + 1..end
    }

    /// An Activity is display-visible only once it has real process members.
    /// Empty markers never become display items.
    pub fn activity_visible(&self, marker: usize) -> bool {
        let members = self.activity_members(marker);
        members.clone().any(|index| {
            matches!(
                self.records[index],
                Record::Reasoning { .. } | Record::Tool { .. }
            )
        })
    }

    /// The visible Activity that owns `record`, if the record is inside one.
    pub fn visible_activity_for(&self, record: usize) -> Option<usize> {
        for &marker in self.activity_markers.iter().rev() {
            if marker >= record {
                continue;
            }
            let members = self.activity_members(marker);
            if record < members.end && self.activity_visible(marker) {
                return Some(marker);
            }
        }
        None
    }

    pub fn activity_expanded(&self, marker: usize) -> bool {
        matches!(
            self.records.get(marker),
            Some(Record::Activity { expanded: true, .. })
        )
    }

    /// Moves a stale selection out of a collapsed Activity onto its marker.
    pub fn normalize_selection(&mut self) {
        if let Some(marker) = self.visible_activity_for(self.selected)
            && !self.activity_expanded(marker)
        {
            self.selected = marker;
        }
    }

    /// Same normalization for the reading anchor; hidden/empty records fall
    /// back to the next visible record so paging never stalls.
    pub fn normalize_anchor(&mut self) {
        let Some(anchor) = self.anchor else {
            return;
        };
        let mut record = anchor.record;
        if let Some(marker) = self.visible_activity_for(record)
            && !self.activity_expanded(marker)
        {
            record = marker;
            self.anchor = Some(Anchor { record, offset: 0 });
            self.visual_anchor = None;
            return;
        }
        let hidden = matches!(self.records.get(record), Some(Record::Activity { .. }))
            && !self.activity_visible(record);
        if hidden {
            record = self.next_visible_record(record).unwrap_or(record);
            self.anchor = Some(Anchor { record, offset: 0 });
            self.visual_anchor = None;
        }
    }

    fn next_visible_record(&self, record: usize) -> Option<usize> {
        let items = self.visible_record_keys();
        items
            .iter()
            .copied()
            .find(|key| *key >= record)
            .or_else(|| items.last().copied())
    }

    /// Flattened display order: record indices plus Activity markers for
    /// collapsed groups. Hidden/empty records are omitted.
    pub fn visible_record_keys(&self) -> Vec<usize> {
        let mut keys = Vec::new();
        let mut index = 0usize;
        while index < self.records.len() {
            match &self.records[index] {
                Record::Activity { .. } => {
                    if self.activity_visible(index) {
                        keys.push(index);
                        if self.activity_expanded(index) {
                            keys.extend(self.activity_members(index));
                        }
                        index = self.activity_members(index).end;
                    } else {
                        index += 1;
                    }
                }
                _ => {
                    keys.push(index);
                    index += 1;
                }
            }
        }
        keys
    }

    /// Applies a new palette and drops cached layouts, whose rendered `Line`s
    /// bake colors. Editor/draft/images, history, records, collapse state,
    /// focus, selection and reading anchors are untouched.
    pub fn set_theme(&mut self, theme: Theme) {
        if self.theme == theme {
            return;
        }
        self.theme = theme;
        for layout in &mut self.layouts {
            *layout = None;
        }
    }

    pub fn should_quit(&self) -> bool {
        self.quit_after_turn && !self.running
    }

    /// Only an explicit End (or scrolling back down to the bottom) resumes
    /// following; new output never steals the position.
    pub fn end(&mut self) {
        self.follow = true;
        self.anchor = None;
        self.unread_output = false;
        self.visual_anchor = None;
    }

    pub fn toggle_help(&mut self) {
        self.help_open = !self.help_open;
        self.help_scroll = 0;
    }

    pub fn notify(&mut self, message: String) {
        self.feedback = Some((message, Instant::now() + Duration::from_secs(3)));
    }

    pub fn expire_feedback(&mut self, now: Instant) {
        if self
            .feedback
            .as_ref()
            .is_some_and(|(_, until)| now >= *until)
        {
            self.feedback = None;
        }
    }

    /// Jumps to the very start of the conversation.
    pub fn home(&mut self) {
        self.follow = false;
        self.anchor = Some(Anchor {
            record: 0,
            offset: 0,
        });
        self.visual_anchor = None;
    }

    /// Toggles tool or reasoning details; manual reasoning expansion is preserved.
    pub fn toggle_selected(&mut self) -> bool {
        self.toggle_record(self.selected)
    }

    pub fn toggle_record(&mut self, index: usize) -> bool {
        let toggled = match self.records.get_mut(index) {
            Some(Record::Activity {
                expanded, manual, ..
            }) => {
                *expanded = !*expanded;
                *manual = true;
                true
            }
            Some(Record::Reasoning {
                expanded,
                auto_collapse,
                ..
            }) => {
                *expanded = !*expanded;
                *auto_collapse = false;
                true
            }
            Some(Record::Tool { expanded, .. }) => {
                *expanded = !*expanded;
                true
            }
            _ => false,
        };
        if toggled {
            self.bump_transcript_revision();
        }
        toggled
    }
}
