use super::*;

mod startup;

#[test]
fn deltas_flush_immediately_and_final_text_does_not_repeat() {
    let mut output = ConsoleOutput::new(Vec::new());
    output
        .apply_event(AgentEvent::ResponseTextDelta {
            item_id: "m".into(),
            delta: "Hello".into(),
        })
        .unwrap();
    assert_eq!(
        String::from_utf8(output.writer.clone()).unwrap(),
        "Agent:\n\nHello"
    );
    for event in [
        AgentEvent::TaskStarted {
            task_id: "task".into(),
            message_id: None,
        },
        AgentEvent::ModelRequestStarted {
            purpose: crate::client::ModelRequestPurpose::Generation,
        },
        AgentEvent::ModelRequestStarted {
            purpose: crate::client::ModelRequestPurpose::Compaction,
        },
    ] {
        output.apply_event(event).unwrap();
    }
    output
        .apply_event(AgentEvent::ResponseTextDelta {
            item_id: "m".into(),
            delta: " world".into(),
        })
        .unwrap();
    output
        .apply_event(AgentEvent::ResponseText {
            item_id: "m".into(),
            text: "Hello world!".into(),
        })
        .unwrap();
    output
        .apply_event(AgentEvent::Usage {
            input_tokens: Some(10),
            output_tokens: Some(3),
        })
        .unwrap();
    output
        .apply_event(AgentEvent::ResponseText {
            item_id: "m".into(),
            text: "Next response".into(),
        })
        .unwrap();
    assert_eq!(
        String::from_utf8(output.writer).unwrap(),
        "Agent:\n\nHello world!\nAgent:\n\nNext response\n"
    );
}

#[test]
fn reasoning_end_closes_the_block_and_final_response_does_not_repeat_it() {
    let mut output = ConsoleOutput::new(Vec::new());
    for event in [
        AgentEvent::ReasoningDelta {
            item_id: "r".into(),
            delta: "Check first".into(),
        },
        AgentEvent::ReasoningCompleted {
            item_id: "r".into(),
        },
        AgentEvent::ResponseTextDelta {
            item_id: "m".into(),
            delta: "Answer".into(),
        },
        AgentEvent::Reasoning {
            item_id: "r".into(),
            text: "Check first".into(),
        },
        AgentEvent::ResponseText {
            item_id: "m".into(),
            text: "Answer".into(),
        },
    ] {
        output.apply_event(event).unwrap();
    }
    assert_eq!(
        String::from_utf8(output.writer).unwrap(),
        "Thinking...:\n\nCheck first\nAgent:\n\nAnswer\n"
    );
}

struct FixtureSessions(
    std::sync::Mutex<std::collections::HashMap<String, crate::client::SessionMetadata>>,
);
impl FixtureSessions {
    fn metadata(&self, id: &str) -> anyhow::Result<crate::client::SessionMetadata> {
        self.0
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("fixture session missing"))
    }
    fn create(&self) -> anyhow::Result<String> {
        let id = crate::client::history::new_id()?;
        self.0.lock().unwrap().insert(
            id.clone(),
            crate::client::tests::metadata(&id, "New session"),
        );
        Ok(id)
    }
    fn rename(&self, id: &str, title: &str) -> anyhow::Result<crate::client::SessionMetadata> {
        let mut entries = self.0.lock().unwrap();
        let item = entries.get_mut(id).unwrap();
        item.title = title.into();
        Ok(item.clone())
    }
    fn list(&self) -> anyhow::Result<Vec<crate::client::SessionSummary>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .values()
            .map(|metadata| crate::client::SessionSummary {
                metadata: metadata.clone(),
                updated_at: 0,
                status: crate::client::history::TaskStatus::Unknown,
                in_use: false,
                unavailable: None,
            })
            .collect())
    }
}
fn session_fixture() -> (std::path::PathBuf, FixtureSessions, String) {
    let root = std::env::temp_dir().join(crate::client::history::new_id().unwrap());
    std::fs::create_dir(&root).unwrap();
    let store = FixtureSessions(Default::default());
    let id = store.create().unwrap();
    (root, store, id)
}
fn idle_state(id: &str) -> crate::client::SessionState {
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

fn snapshot_event(store: &FixtureSessions, id: &str) -> SessionEvent {
    let metadata = store.0.lock().unwrap()[id].clone();
    SessionEvent::Opened {
        state: idle_state(id),
        metadata,
        replay: crate::client::history::History::default(),
        context: AgentEvent::ContextUpdated {
            model: "test".into(),
            context_tokens: None,
            context_window: Some(262144),
            input_limit: Some(253952),
        },
        stream: None,
    }
}

fn association(id: &str, task: &str) -> crate::client::Association {
    crate::client::Association {
        session_id: id.to_owned(),
        task_id: task.to_owned(),
        message_id: format!("{task}-message"),
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

#[tokio::test]
async fn console_local_commands_use_session_operations_and_never_send_model_messages() {
    let (root, store, id) = session_fixture();
    let script = format!(
        "/help\n/rename Compiler work\n/sessions\n/new\n/resume {id}\n/resume missing\n/resuem\n"
    );
    let (tx, mut requests) = tokio::sync::mpsc::channel::<SessionRequest>(32);
    let (events, rx): (
        crate::session::SessionEventSender,
        crate::session::SessionEventReceiver,
    ) = crate::session::channel::channel();
    events.send(snapshot_event(&store, &id)).await.unwrap();
    let backend = {
        let events = events.clone();
        async move {
            let mut operations = Vec::new();
            while let Some(request) = requests.recv().await {
                let request_id = request.request_id;
                let event = match request.command {
                    SessionCommand::Send { .. } => {
                        panic!("local command reached model message path")
                    }
                    SessionCommand::Rename { session_id, title } => {
                        operations.push("rename");
                        SessionEvent::Renamed(store.rename(&session_id, &title).unwrap())
                    }
                    SessionCommand::List => {
                        operations.push("list");
                        SessionEvent::Listed(store.list().unwrap())
                    }
                    SessionCommand::Create => {
                        operations.push("create");
                        let new = store.create().unwrap();
                        snapshot_event(&store, &new)
                    }
                    SessionCommand::Open { session_id } => {
                        operations.push("open");
                        if session_id == "missing" {
                            SessionEvent::Error {
                                operation: "open",
                                error: crate::client::RuntimeError::new(
                                    "not_found",
                                    "session missing",
                                ),
                            }
                        } else {
                            snapshot_event(&store, &session_id)
                        }
                    }
                    SessionCommand::Cancel { .. } => SessionEvent::Error {
                        operation: "cancel",
                        error: crate::client::RuntimeError::new("not_running", "missing"),
                    },
                };
                events.send_reply(request_id, None, event).await.unwrap();
            }
            drop(events);
            operations
        }
    };
    drop(events);
    let mut output = Vec::new();
    let frontend = run_with_io(
        tx,
        rx,
        BufReader::new(script.as_bytes()),
        &mut output,
        false,
    );
    let (result, operations) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(frontend, backend)
    })
    .await
    .unwrap();
    result.unwrap();
    assert_eq!(operations, vec!["rename", "list", "create", "open", "open"]);
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("Local commands:"));
    assert!(text.contains("Compiler work"));
    assert!(text.contains("Input kept: /resume missing"));
    assert!(text.contains("Input kept: /resuem"));
    assert!(
        !text.contains("Tool started:"),
        "local commands must remain local"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn console_can_handle_commands_during_a_job_and_drain_piped_messages_after_eof() {
    let (root, store, id) = session_fixture();
    let script = "first job\n/help\n/sessions\n/rename Active job\nsecond job\n";
    let (tx, mut requests) = tokio::sync::mpsc::channel::<SessionRequest>(32);
    let (events, rx): (
        crate::session::SessionEventSender,
        crate::session::SessionEventReceiver,
    ) = crate::session::channel::channel();
    events.send(snapshot_event(&store, &id)).await.unwrap();
    let backend = {
        let events = events.clone();
        let id = id.clone();
        async move {
            let mut sent = Vec::new();
            while let Some(request) = requests.recv().await {
                let request_id = request.request_id;
                match request.command {
                    SessionCommand::Send {
                        session_id,
                        message,
                    } => {
                        assert_eq!(session_id, id);
                        sent.push(message.text);
                        let assoc = association(&id, &format!("task-{}", sent.len()));
                        events
                            .send_reply(request_id, Some(assoc.clone()), SessionEvent::Accepted)
                            .await
                            .unwrap();
                        if sent.len() == 1 {
                            events
                                .send_associated(
                                    assoc.clone(),
                                    SessionEvent::Agent {
                                        session_id: id.clone(),
                                        event: AgentEvent::ResponseTextDelta {
                                            item_id: "m".into(),
                                            delta: "Working".into(),
                                        },
                                    },
                                )
                                .await
                                .unwrap();
                            events
                                .send_associated(
                                    assoc,
                                    SessionEvent::Finished(finished("completed")),
                                )
                                .await
                                .unwrap();
                        } else {
                            events
                                .send_associated(
                                    assoc,
                                    SessionEvent::Finished(finished("completed")),
                                )
                                .await
                                .unwrap();
                        }
                    }
                    SessionCommand::List => {
                        assert_eq!(sent.len(), 1);
                        events
                            .send_reply(
                                request_id,
                                None,
                                SessionEvent::Listed(store.list().unwrap()),
                            )
                            .await
                            .unwrap();
                    }
                    SessionCommand::Rename { session_id, title } => {
                        assert_eq!(sent.len(), 1);
                        let metadata = store.rename(&session_id, &title).unwrap();
                        events
                            .send_reply(
                                request_id,
                                None,
                                SessionEvent::TitleUpdated(metadata.clone()),
                            )
                            .await
                            .unwrap();
                        events
                            .send_reply(request_id, None, SessionEvent::Renamed(metadata))
                            .await
                            .unwrap();
                    }
                    _ => panic!("unexpected request"),
                }
            }
            drop(events);
            sent
        }
    };
    drop(events);
    let mut output = Vec::new();
    let frontend = run_with_io(
        tx,
        rx,
        BufReader::new(script.as_bytes()),
        &mut output,
        false,
    );
    let (result, sent) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(frontend, backend)
    })
    .await
    .unwrap();
    result.unwrap();
    assert_eq!(sent, vec!["first job", "second job"]);
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("Local commands:"));
    assert!(text.contains("Active job"));
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn history_restored_repairs_authoritative_body_before_one_finished() {
    use crate::client::history::{History, HistoryEntry, TaskStatus};
    use crate::client::types::{InputItem, InputMessageContent, MessageRole};

    let (root, store, id) = session_fixture();
    let task = "task-repair";
    let assoc = association(&id, task);
    let replay = History {
        history: vec![
            HistoryEntry::TaskStarted {
                task_id: task.to_owned(),
                message_id: Some(assoc.message_id.clone()),
                configuration: None,
            },
            HistoryEntry::Messages {
                items: vec![
                    InputItem::Message {
                        role: MessageRole::User,
                        content: InputMessageContent::Text("canonical question".into()),
                    },
                    InputItem::Message {
                        role: MessageRole::Assistant,
                        content: InputMessageContent::Text("authoritative answer".into()),
                    },
                ],
            },
            HistoryEntry::TaskFinished {
                task_id: task.to_owned(),
                status: TaskStatus::Failed,
                error: Some("canonical failure".into()),
            },
        ],

        status: TaskStatus::Failed,
    };
    let (tx, mut requests) = tokio::sync::mpsc::channel::<SessionRequest>(32);
    let (events, rx): (
        crate::session::SessionEventSender,
        crate::session::SessionEventReceiver,
    ) = crate::session::channel::channel();
    events.send(snapshot_event(&store, &id)).await.unwrap();
    let backend = {
        let events = events.clone();
        async move {
            let request = requests.recv().await.expect("send request");
            assert!(matches!(request.command, SessionCommand::Send { .. }));
            events
                .send_reply(
                    request.request_id,
                    Some(assoc.clone()),
                    SessionEvent::Accepted,
                )
                .await
                .unwrap();
            events
                .send_associated(
                    assoc.clone(),
                    SessionEvent::Agent {
                        session_id: id.clone(),
                        event: AgentEvent::ResponseTextDelta {
                            item_id: "m".into(),
                            delta: "partial live".into(),
                        },
                    },
                )
                .await
                .unwrap();
            events
                .send_associated(
                    assoc.clone(),
                    SessionEvent::HistoryRestored {
                        metadata: store.metadata(&id).unwrap(),
                        replay,
                        context: AgentEvent::ContextUpdated {
                            model: "test".into(),
                            context_tokens: None,
                            context_window: Some(262_144),
                            input_limit: Some(253_952),
                        },
                        state: idle_state(&id),
                    },
                )
                .await
                .unwrap();
            events
                .send_associated(
                    assoc,
                    SessionEvent::Finished(crate::client::TaskFinished {
                        status: "failed".into(),
                        error: Some(crate::client::RuntimeError::new(
                            "model_error",
                            "late failure",
                        )),
                        persistence: Some("unconfirmed".into()),
                        history_required: true,
                    }),
                )
                .await
                .unwrap();
        }
    };
    drop(events);
    let mut output = Vec::new();
    let frontend = run_with_io(
        tx,
        rx,
        BufReader::new("hello\n".as_bytes()),
        &mut output,
        false,
    );
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(frontend, backend)
    })
    .await
    .expect("console wedged")
    .0
    .unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("History repaired"), "{text}");
    assert!(
        text.contains("authoritative answer"),
        "canonical body must be rendered: {text}"
    );
    assert!(text.contains("late failure"), "{text}");
    assert!(text.contains("model_error"), "{text}");
    assert!(text.contains("unconfirmed"), "{text}");
    assert_eq!(
        text.matches("Task failed:").count(),
        1,
        "Finished reports the single terminal line: {text}"
    );
    assert_eq!(
        text.matches("History repaired").count(),
        1,
        "repair is rendered once: {text}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn history_restored_interrupted_is_reported_once_by_finished() {
    use crate::client::history::{History, HistoryEntry, TaskStatus};
    use crate::client::types::{InputItem, InputMessageContent, MessageRole};

    let (root, store, id) = session_fixture();
    let task = "task-interrupted";
    let assoc = association(&id, task);
    let replay = History {
        history: vec![
            HistoryEntry::TaskStarted {
                task_id: task.to_owned(),
                message_id: Some(assoc.message_id.clone()),
                configuration: None,
            },
            HistoryEntry::Messages {
                items: vec![
                    InputItem::Message {
                        role: MessageRole::User,
                        content: InputMessageContent::Text("interrupted question".into()),
                    },
                    InputItem::Message {
                        role: MessageRole::Assistant,
                        content: InputMessageContent::Text("interrupted authoritative body".into()),
                    },
                ],
            },
            HistoryEntry::TaskFinished {
                task_id: task.to_owned(),
                status: TaskStatus::Interrupted,
                error: None,
            },
        ],

        status: TaskStatus::Interrupted,
    };
    let (tx, mut requests) = tokio::sync::mpsc::channel::<SessionRequest>(32);
    let (events, rx): (
        crate::session::SessionEventSender,
        crate::session::SessionEventReceiver,
    ) = crate::session::channel::channel();
    events.send(snapshot_event(&store, &id)).await.unwrap();
    let backend = {
        let events = events.clone();
        async move {
            let request = requests.recv().await.expect("send request");
            events
                .send_reply(
                    request.request_id,
                    Some(assoc.clone()),
                    SessionEvent::Accepted,
                )
                .await
                .unwrap();
            events
                .send_associated(
                    assoc.clone(),
                    SessionEvent::HistoryRestored {
                        metadata: store.metadata(&id).unwrap(),
                        replay,
                        context: AgentEvent::ContextUpdated {
                            model: "test".into(),
                            context_tokens: None,
                            context_window: Some(262_144),
                            input_limit: Some(253_952),
                        },
                        state: idle_state(&id),
                    },
                )
                .await
                .unwrap();
            events
                .send_associated(
                    assoc,
                    SessionEvent::Finished(crate::client::TaskFinished {
                        status: "interrupted".into(),
                        error: None,
                        persistence: Some("unconfirmed".into()),
                        history_required: true,
                    }),
                )
                .await
                .unwrap();
        }
    };
    drop(events);
    let mut output = Vec::new();
    let frontend = run_with_io(
        tx,
        rx,
        BufReader::new("hello\n".as_bytes()),
        &mut output,
        false,
    );
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(frontend, backend)
    })
    .await
    .expect("console wedged")
    .0
    .unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("interrupted authoritative body"), "{text}");
    assert_eq!(
        text.matches("Task interrupted.").count(),
        1,
        "Finished is the sole interrupted terminal: {text}"
    );
    assert_eq!(text.matches("Task failed:").count(), 0, "{text}");
    assert!(text.contains("unconfirmed"), "{text}");
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn closed_command_mailbox_ends_console_without_retry_loop() {
    let (root, store, id) = session_fixture();
    let (tx, rx) = tokio::sync::mpsc::channel::<SessionRequest>(1);
    drop(rx);
    let (events, events_rx): (
        crate::session::SessionEventSender,
        crate::session::SessionEventReceiver,
    ) = crate::session::channel::channel();
    events.send(snapshot_event(&store, &id)).await.unwrap();
    let mut output = Vec::new();
    let frontend = run_with_io(
        tx,
        events_rx,
        BufReader::new("hello after close\n".as_bytes()),
        &mut output,
        false,
    );
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), frontend)
        .await
        .expect("closed mailbox must not hang EOF");
    result.unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("Session output is closed."), "{text}");
    assert!(
        !text.contains("queue is full"),
        "closed mailbox must not look like a retryable full queue: {text}"
    );
    std::fs::remove_dir_all(root).unwrap();
}
