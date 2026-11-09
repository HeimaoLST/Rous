use std::{
    collections::{HashMap, VecDeque},
    io::{IsTerminal, Write},
};

use tokio::io::{AsyncBufRead, AsyncBufReadExt, BufReader};
use tokio::sync::mpsc::Sender;

use crate::client::AgentEvent;
use crate::frontend::session_requests::{PendingRequest, RequestSendError, SessionRequests};
use crate::{
    client::{
        Association,
        history::{HistoryEntry, TaskStatus},
        types::{InputItem, InputMessageContent, MessageRole},
    },
    command::{self, Command, ParsedInput},
    input::UserInput,
    session::{SessionCommand, SessionEvent, SessionEventReceiver, SessionRequest},
};

/// Independent plain-text front end.
///
/// It owns stdin/stdout and connects to the agent worker through the same
/// channels the TUI will use. On stdin EOF it closes the input channel but keeps
/// consuming events until the worker ends, so a piped single message still gets
/// its answer printed.
pub async fn run(
    input: Sender<SessionRequest>,
    events: SessionEventReceiver,
) -> anyhow::Result<()> {
    run_with_io(
        input,
        events,
        BufReader::new(tokio::io::stdin()),
        std::io::stdout(),
        std::io::stdin().is_terminal(),
    )
    .await
}

async fn run_with_io<R: AsyncBufRead + Unpin, W: Write>(
    input: Sender<SessionRequest>,
    mut events: SessionEventReceiver,
    reader: R,
    writer: W,
    interactive: bool,
) -> anyhow::Result<()> {
    let mut output = ConsoleOutput::new(writer);
    let mut requests = SessionRequests::new(input);
    let mut lines = reader.lines();
    let mut session_id: Option<String> = None;
    let mut association: Option<Association> = None;
    let mut running = false;
    let mut sending = false;
    let mut eof = false;
    let mut queue = VecDeque::<UserInput>::new();
    let mut pending: Option<(u64, Command, String)> = None;
    let mut ctrl_c = Box::pin(tokio::signal::ctrl_c());
    loop {
        if !running
            && !sending
            && pending.is_none()
            && let Some(message) = queue.pop_front()
        {
            let Some(id) = session_id.clone() else {
                queue.push_front(message);
                continue;
            };
            let pending_request = PendingRequest::Send {
                session_id: id.clone(),
                message: message.clone(),
            };
            let command = SessionCommand::Send {
                session_id: id,
                message: message.clone(),
            };
            match requests.try_send(command, pending_request) {
                Ok(_) => sending = true,
                Err(RequestSendError::Full) => {
                    queue.push_front(message);
                    output.feedback("Send queue is full. Input kept.", None)?;
                }
                Err(RequestSendError::Closed) => {
                    output.feedback("Session output is closed.", None)?;
                    break;
                }
            }
        }
        if eof && queue.is_empty() && pending.is_none() && !sending {
            requests.close();
        }
        tokio::select! {
            _ = &mut ctrl_c => {
                ctrl_c = Box::pin(tokio::signal::ctrl_c());
                if let Some(association) = &association {
                    let command = SessionCommand::Cancel {
                        session_id: association.session_id.clone(),
                        task_id: association.task_id.clone(),
                    };
                    let pending = PendingRequest::Cancel {
                        session_id: association.session_id.clone(),
                        task_id: association.task_id.clone(),
                    };
                    match requests.try_send(command, pending) {
                        Ok(_) => output.feedback("Cancellation requested.", None)?,
                        Err(RequestSendError::Full) => {
                            output.feedback("Cancel queue is full; task still running.", None)?
                        }
                        Err(RequestSendError::Closed) => break,
                    }
                } else {
                    break;
                }
            }
            line = lines.next_line(), if !eof && session_id.is_some() && pending.is_none() && !sending => {
                let Some(line) = line? else { eof = true; continue; };
                match command::parse(&line, false) {
                    Ok(ParsedInput::Message(text)) => {
                        if interactive && running {
                            output.feedback("Agent is busy. Wait for the current task to finish.", Some(&line))?;
                        } else {
                            queue.push_back(text.into());
                        }
                    }
                    Ok(ParsedInput::Command(command)) => {
                        if command == Command::Help {
                            output.close_block()?;
                            writeln!(output.writer, "Local commands:")?;
                            for definition in command::DEFINITIONS { writeln!(output.writer, "  {}  {}", definition.usage, definition.description)?; }
                            writeln!(output.writer, "Use // to send literal slash text. Multiline input stays a message.")?;
                            output.prompt()?;
                        } else {
                            let Some(id) = session_id.as_deref() else { continue; };
                            let Some(request) = command.request(id) else { continue; };
                            match requests.try_send(request, PendingRequest::Command {
                                        draft: String::new(),
                                        session_id: session_id.clone(),
                                    }) {
                                Ok(request_id) => {
                                    pending = Some((request_id, command, line));
                                }
                                Err(RequestSendError::Full) => {
                                    output.feedback("Command queue is full. Input kept.", Some(&line))?;
                                }
                                Err(RequestSendError::Closed) => break,
                            }
                        }
                    }
                    Err(error) => output.feedback(&error.to_string(), Some(&line))?,
                }
            }
            packet = events.recv() => {
                let Some(packet) = packet else { break; };
                let request_id = packet.request_id;
                let resolution = request_id.and_then(|id| requests.take(id));
                let mut error_reported = false;
                let packet_association = packet.association.clone();
                let event = packet.event;
                match &resolution {
                    Some(PendingRequest::Send { message, .. }) if matches!(event, SessionEvent::Error { .. }) => {
                        sending = false;
                        error_reported = true;
                        output.feedback("Send rejected. Input kept.", Some(&message.text))?;
                    }
                    Some(PendingRequest::Command { .. }) => {
                        if matches!(event, SessionEvent::Error { .. }) {
                            if let Some((_, _, line)) = pending.take() {
                                output.feedback("Command failed. Input kept.", Some(&line))?;
                            }
                            error_reported = true;
                        } else {
                            pending = None;
                        }
                    }
                    Some(PendingRequest::Control) => {
                        pending = None;
                        if matches!(event, SessionEvent::Error { .. }) {
                            error_reported = true;
                        }
                    }
                    Some(PendingRequest::Cancel { .. }) if matches!(event, SessionEvent::Error { .. }) => {
                        output.feedback("Cancel request failed.", None)?;
                        error_reported = true;
                    }
                    _ => {}
                }
                match event {
                    SessionEvent::Opened { metadata, replay, stream, state, .. } => {
                        output.close_block()?;
                        writeln!(output.writer, "Session: {} ({})", metadata.title, metadata.id)?;
                        output.restore(&replay)?;
                        if matches!(replay.status, TaskStatus::Interrupted | TaskStatus::Unknown) && replay.has_task_history() {
                            writeln!(output.writer, "Previous task: {:?}. Nothing was re-executed.", replay.status)?;
                        }
                        session_id = Some(metadata.id);
                        running = stream.is_some() || state.active_task.is_some();
                        association = stream;
                        sending = false;
                        output.prompt()?;
                    }
                    SessionEvent::HistoryRestored {
                        metadata,
                        replay,
                        ..
                    } => {
                        let matches = packet_association.as_ref().is_some_and(|packet| {
                            session_id.as_deref() == Some(metadata.id.as_str())
                                && association
                                    .as_ref()
                                    .is_some_and(|current| same_association(current, packet))
                        });
                        if matches {
                            let task_id = packet_association
                                .as_ref()
                                .map(|item| item.task_id.clone())
                                .unwrap_or_default();
                            output.close_block()?;
                            writeln!(
                                output.writer,
                                "History repaired for {} (task {task_id}):",
                                metadata.id
                            )?;
                            let start = replay.history.iter().position(|record| {
                                matches!(
                                    record,
                                    HistoryEntry::TaskStarted { task_id: id, .. } if *id == task_id
                                )
                            });
                            if let Some(start) = start {
                                output.restore_slice(&replay.history[start..], start, false)?;
                            } else {
                                output.restore(&replay)?;
                            }
                            output.prompt()?;
                        }
                    }
                    SessionEvent::Listed(sessions) => {
                        output.close_block()?;
                        writeln!(output.writer, "Sessions:")?;
                        for session in sessions {
                            let state = if session.unavailable.is_some() { "unavailable" } else if session.in_use { "in use" } else { "available" };
                            writeln!(output.writer, "  {}  {}  [{state}]  {}", session.metadata.id, session.metadata.title, session.metadata.working_directory.display())?;
                        }
                        output.prompt()?;
                    }
                    SessionEvent::Renamed(metadata) => {
                        output.feedback(&format!("Session renamed: {}", metadata.title), None)?;
                    }
                    SessionEvent::TitleUpdated(_) => {}
                    SessionEvent::Accepted => {
                        sending = false;
                        output.reset_task();
                        if packet_association.as_ref().is_some_and(|item| session_id.as_deref() == Some(item.session_id.as_str())) {
                            association = packet_association.clone();
                            running = true;
                        }
                    }
                    SessionEvent::Agent { event, .. } => {
                        if matches!(event, AgentEvent::TurnCompleted | AgentEvent::TurnInterrupted | AgentEvent::TurnFailed { .. }) {
                            // Finished owns the single terminal.
                        } else if packet_association.as_ref().is_some_and(|packet| {
                            session_id.as_deref() == Some(packet.session_id.as_str())
                                && association
                                    .as_ref()
                                    .is_some_and(|current| same_association(current, packet))
                        }) {
                            output.apply_event(event)?;
                        }
                    }
                    SessionEvent::Finished(finished) => {
                        let matches_current = packet_association.as_ref().is_some_and(|packet| {
                            session_id.as_deref() == Some(packet.session_id.as_str())
                                && association
                                    .as_ref()
                                    .is_some_and(|current| same_association(current, packet))
                        });
                        if matches_current {
                            running = false;
                            sending = false;
                            output.close_block()?;
                            output.reset_task();
                            match finished.status.as_str() {
                                "completed" => {}
                                "interrupted" => writeln!(output.writer, "Task interrupted.")?,
                                _ => {
                                    let (message, code) = finished.error.as_ref().map_or_else(
                                        || ("Task failed.".to_owned(), "unknown"),
                                        |error| (error.message.clone(), error.code.as_str()),
                                    );
                                    writeln!(output.writer, "Task failed: {message} ({code})")?;
                                }
                            }
                            if let Some(persistence) = finished.persistence {
                                writeln!(output.writer, "Task persistence: {persistence}")?;
                            }
                            association = None;
                            if !eof { output.prompt()?; }
                        }
                    }
                    SessionEvent::Cancelled { status } => {
                        let matches = match &resolution {
                            Some(PendingRequest::Cancel { session_id, task_id }) => association
                                .as_ref()
                                .is_some_and(|current| {
                                    current.session_id == *session_id && current.task_id == *task_id
                                }),
                            _ => false,
                        };
                        if matches {
                            if status == "not_running" {
                                output.feedback("No running task to cancel.", None)?;
                            } else {
                                output.feedback("Cancellation requested; waiting for the task to finish.", None)?;
                            }
                        }
                    }
                    SessionEvent::Error { operation, error } => {
                        if !error_reported {
                            output.feedback(&format!("{operation}: {}", error.message), None)?;
                        }
                    }
                }
            }
        }
    }
    output.writer.flush()?;
    Ok(())
}

fn same_association(left: &Association, right: &Association) -> bool {
    left.session_id == right.session_id
        && left.task_id == right.task_id
        && left.message_id == right.message_id
}

struct ConsoleOutput<W> {
    writer: W,
    active: Option<(bool, String)>,
    streamed: HashMap<(bool, String), String>,
}

impl<W: Write> ConsoleOutput<W> {
    fn new(writer: W) -> Self {
        Self {
            writer,
            active: None,
            streamed: HashMap::new(),
        }
    }

    fn prompt(&mut self) -> std::io::Result<()> {
        write!(self.writer, "User:\t")?;
        self.writer.flush()
    }

    fn feedback(&mut self, message: &str, kept: Option<&str>) -> std::io::Result<()> {
        self.close_block()?;
        writeln!(self.writer, "{message}")?;
        if let Some(text) = kept {
            writeln!(self.writer, "Input kept: {text}")?;
        }
        self.prompt()
    }

    fn restore(&mut self, replay: &crate::client::history::History) -> std::io::Result<()> {
        self.restore_slice(&replay.history, 0, true)
    }

    /// Appends canonical records from one slice. Used for initial history and
    /// for associated append-only HistoryRestored repair of one task.
    fn restore_slice(
        &mut self,
        records: &[HistoryEntry],
        base: usize,
        include_terminal: bool,
    ) -> std::io::Result<()> {
        let mut tools = HashMap::new();
        for (offset, record) in records.iter().enumerate() {
            let index = base + offset;
            match record {
                HistoryEntry::Messages { items } => {
                    for (part, item) in items.iter().enumerate() {
                        let item_id = format!("history-{index}-{part}");
                        match item {
                            InputItem::Message { role, content } => {
                                let text = match content {
                                    InputMessageContent::Text(text) => text.clone(),
                                    InputMessageContent::Parts(parts) => parts
                                        .iter()
                                        .map(|part| match part {
                                            crate::client::types::InputContentPart::Text {
                                                text,
                                            } => text.clone(),
                                            crate::client::types::InputContentPart::Image {
                                                ..
                                            } => "[image]".to_owned(),
                                        })
                                        .collect::<Vec<_>>()
                                        .join("\n"),
                                };
                                match role {
                                    MessageRole::User => writeln!(self.writer, "User: {text}")?,
                                    MessageRole::Assistant => {
                                        self.complete(false, item_id, text)?
                                    }
                                    _ => {}
                                }
                            }
                            InputItem::Reasoning { content, .. } => self.complete(
                                true,
                                item_id,
                                content.iter().map(|part| part.text.as_str()).collect(),
                            )?,
                            InputItem::ToolCall {
                                call_id,
                                name,
                                arguments,
                            } => {
                                tools.insert(call_id.clone(), name.clone());
                                self.apply_event(AgentEvent::ToolStarted {
                                    call_id: call_id.clone(),
                                    name: name.clone(),
                                    arguments: arguments.clone(),
                                })?;
                            }
                            InputItem::ToolResult { call_id, output } => {
                                self.apply_event(AgentEvent::ToolFinished {
                                    call_id: call_id.clone(),
                                    name: tools.remove(call_id).unwrap_or_default(),
                                    output: serde_json::from_str(output).unwrap_or_else(|_| {
                                        serde_json::Value::String(output.clone())
                                    }),
                                })?
                            }
                            _ => {}
                        }
                    }
                }
                HistoryEntry::Compaction {
                    tokens_before,
                    tokens_after,
                    usage,
                    ..
                } => self.apply_event(AgentEvent::Compaction {
                    tokens_before: *tokens_before,
                    tokens_after: *tokens_after,
                    summary_output_tokens: usage.as_ref().and_then(|usage| usage.output_tokens),
                })?,
                HistoryEntry::TaskFinished {
                    error: Some(error), ..
                } if include_terminal => writeln!(self.writer, "Task failed: {error}")?,
                _ => {}
            }
        }
        Ok(())
    }

    fn reset_task(&mut self) {
        self.active = None;
        self.streamed.clear();
    }

    fn close_block(&mut self) -> std::io::Result<()> {
        if self.active.take().is_some() {
            writeln!(self.writer)?;
        }
        Ok(())
    }

    fn delta(&mut self, reasoning: bool, item_id: String, delta: String) -> std::io::Result<()> {
        if delta.is_empty() {
            return Ok(());
        }
        let key = (reasoning, item_id);
        if self.active.as_ref() != Some(&key) {
            self.close_block()?;
            writeln!(
                self.writer,
                "{}:\n",
                if reasoning { "Thinking..." } else { "Agent" }
            )?;
            self.active = Some(key.clone());
        }
        write!(self.writer, "{delta}")?;
        self.writer.flush()?;
        self.streamed.entry(key).or_default().push_str(&delta);
        Ok(())
    }

    fn complete(&mut self, reasoning: bool, item_id: String, text: String) -> std::io::Result<()> {
        let key = (reasoning, item_id.clone());
        let remaining = match self.streamed.remove(&key) {
            Some(shown) => text.strip_prefix(&shown).unwrap_or(&text).to_owned(),
            None => text,
        };
        self.delta(reasoning, item_id, remaining)?;
        self.streamed.remove(&key);
        self.close_block()
    }

    fn apply_event(&mut self, event: AgentEvent) -> std::io::Result<()> {
        match event {
            AgentEvent::TaskStarted { .. } | AgentEvent::ModelRequestStarted { .. } => {}
            AgentEvent::ResponseTextDelta { item_id, delta } => {
                self.delta(false, item_id, delta)?
            }
            AgentEvent::ReasoningDelta { item_id, delta } => self.delta(true, item_id, delta)?,
            AgentEvent::ResponseText { item_id, text } => self.complete(false, item_id, text)?,
            AgentEvent::Reasoning { item_id, text } => self.complete(true, item_id, text)?,
            AgentEvent::ReasoningCompleted { item_id } => {
                if self.active.as_ref() == Some(&(true, item_id)) {
                    self.close_block()?;
                }
            }
            AgentEvent::ToolStarted { name, .. } => {
                self.close_block()?;
                writeln!(self.writer, "Tool started: {name}")?;
            }
            AgentEvent::ToolFinished { name, call_id, .. } => {
                writeln!(self.writer, "Tool finished: {name} ({call_id})")?;
            }
            AgentEvent::Compaction {
                tokens_before,
                tokens_after,
                summary_output_tokens,
            } => {
                self.close_block()?;
                let summary = summary_output_tokens
                    .map(|tokens| format!(", summary output: {tokens}"))
                    .unwrap_or_default();
                writeln!(
                    self.writer,
                    "Context compacted: ~{tokens_before} → ~{tokens_after} input tokens{summary}"
                )?;
            }
            AgentEvent::Usage { .. } | AgentEvent::TurnCompleted => {
                self.close_block()?;
                self.streamed.clear();
            }
            AgentEvent::TurnInterrupted => {
                self.close_block()?;
                self.streamed.clear();
                eprintln!("Task interrupted.");
            }
            AgentEvent::TurnFailed { message } => {
                self.close_block()?;
                self.streamed.clear();
                eprintln!("Task failed: {message}");
            }
            AgentEvent::ContextUpdated { .. } => {}
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../tests/console/mod.rs"]
mod tests;
