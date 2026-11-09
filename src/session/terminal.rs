use super::SessionEventSender;
use crate::{
    client::{
        AgentEvent, Association, Client, RuntimeError, RuntimeProcess, SessionMetadata,
        SessionState, SessionSummary, TaskFinished, history::History, types::Snapshot,
    },
    input::UserInput,
};
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinSet;

pub enum SessionCommand {
    Send {
        session_id: String,
        message: UserInput,
    },
    List,
    Create,
    Open {
        session_id: String,
    },
    Rename {
        session_id: String,
        title: String,
    },
    Cancel {
        session_id: String,
        task_id: String,
    },
}
pub struct SessionRequest {
    pub request_id: u64,
    pub command: SessionCommand,
}
#[derive(Serialize)]
pub enum SessionEvent {
    Opened {
        metadata: SessionMetadata,
        replay: History,
        context: AgentEvent,
        state: SessionState,
        stream: Option<Association>,
    },
    HistoryRestored {
        metadata: SessionMetadata,
        replay: History,
        context: AgentEvent,
        state: SessionState,
    },
    Listed(Vec<SessionSummary>),
    Renamed(SessionMetadata),
    TitleUpdated(SessionMetadata),
    Agent {
        session_id: String,
        event: AgentEvent,
    },
    Accepted,
    Finished(TaskFinished),
    Cancelled {
        status: String,
    },
    Error {
        operation: &'static str,
        error: RuntimeError,
    },
}
pub struct SessionManager {
    process: RuntimeProcess,
    initial_session: Option<String>,
}
type Delivery = Arc<Mutex<HashMap<String, Association>>>;
impl SessionCommand {
    fn operation(&self) -> &'static str {
        match self {
            Self::Send { .. } => "send",
            Self::List => "list",
            Self::Create => "create",
            Self::Open { .. } => "open",
            Self::Rename { .. } => "rename",
            Self::Cancel { .. } => "cancel",
        }
    }
}
impl SessionManager {
    pub fn new(process: RuntimeProcess, initial_session: Option<String>) -> Self {
        Self {
            process,
            initial_session,
        }
    }
    pub async fn run(
        mut self,
        mut input: mpsc::Receiver<SessionRequest>,
        output: SessionEventSender,
    ) -> Result<()> {
        let client = self.process.client.clone();
        let delivery: Delivery = Default::default();
        let startup = match self.initial_session.take() {
            Some(session_id) => SessionCommand::Open { session_id },
            None => SessionCommand::Create,
        };
        if let Err(error) = execute(&client, startup, None, &output, &delivery).await {
            let _ = output.send(error_event("startup", error.clone())).await;
            let _ = client.call::<Value>("runtime.shutdown", json!({})).await;
            self.process.finish().await?;
            return Err(error.into());
        }
        client
            .call::<Value>("runtime.metadata.subscribe", json!({}))
            .await?;
        let (unused, empty) = mpsc::unbounded_channel();
        drop(unused);
        let events = std::mem::replace(&mut self.process.events, empty);
        let mut forwarder = tokio::spawn(forward_events(
            client.clone(),
            events,
            output.clone(),
            delivery.clone(),
        ));
        let mut jobs = JoinSet::new();
        let mut closed = false;
        let mut forwarded = false;
        let mut forwarding_error = None;
        loop {
            if closed && jobs.is_empty() {
                break;
            }
            tokio::select! {
                biased;
                _=output.closed(),if !closed=>{closed=true;input.close();}
                result=jobs.join_next(),if !jobs.is_empty()=>{result.expect("nonempty jobs")?;}
                result=&mut forwarder,if !forwarded=>{forwarded=true;forwarding_error=result?.err();closed=true;input.close();}
                request=input.recv(),if !closed=>{
                    match request {
                        Some(request) if jobs.len()<8=>{
                            let client=client.clone();let output=output.clone();let delivery=delivery.clone();
                            jobs.spawn(async move{
                                let operation=request.command.operation();
                                if let Err(error)=execute(&client,request.command,Some(request.request_id),&output,&delivery).await {
                                    let _=output.send_reply(request.request_id,None,error_event(operation,error)).await;
                                }
                            });
                        }
                        Some(request)=>{let _=output.try_send_reply(request.request_id,error_event(request.command.operation(),RuntimeError::new("busy","too many terminal operations")));},
                        None=>closed=true,
                    }
                }
            }
        }
        let _ = client.call::<Value>("runtime.shutdown", json!({})).await;
        self.process.finish().await?;
        if !forwarded {
            forwarder.await??;
        }
        if let Some(error) = forwarding_error {
            return Err(error);
        }
        Ok(())
    }
}
fn error_event(operation: &'static str, error: RuntimeError) -> SessionEvent {
    SessionEvent::Error { operation, error }
}
async fn snapshot(
    client: &Client,
    id: &str,
    stream: Option<Association>,
    restore: bool,
) -> Result<SessionEvent, RuntimeError> {
    let params = match &stream {
        Some(a) => json!({"session_id":id,"before_task_id":a.task_id}),
        None => json!({"session_id":id}),
    };
    let snapshot: Snapshot = client.call("session.snapshot", params).await?;
    if snapshot.session_id != id {
        return Err(RuntimeError::new(
            "invalid_response",
            "snapshot returned another session",
        ));
    }
    let status = if stream.is_some() {
        snapshot
            .records
            .iter()
            .rev()
            .find_map(|record| match &record.record {
                crate::client::history::HistoryEntry::TaskFinished { status, .. } => {
                    Some(status.clone())
                }
                crate::client::history::HistoryEntry::TaskStarted { .. } => {
                    Some(Default::default())
                }
                _ => None,
            })
            .unwrap_or_default()
    } else {
        serde_json::from_value(json!(snapshot.session.status)).unwrap_or_default()
    };
    let replay = History {
        history: snapshot
            .records
            .into_iter()
            .map(|record| record.record)
            .collect(),
        status,
    };
    let context = AgentEvent::ContextUpdated {
        model: snapshot.context.model,
        context_tokens: snapshot.context.context_tokens,
        context_window: snapshot.context.context_window,
        input_limit: snapshot.context.input_limit,
    };
    Ok(if restore {
        SessionEvent::HistoryRestored {
            metadata: snapshot.metadata,
            replay,
            context,
            state: snapshot.session,
        }
    } else {
        SessionEvent::Opened {
            metadata: snapshot.metadata,
            replay,
            context,
            state: snapshot.session,
            stream,
        }
    })
}
async fn execute(
    client: &Client,
    command: SessionCommand,
    request_id: Option<u64>,
    output: &SessionEventSender,
    delivery: &Delivery,
) -> Result<(), RuntimeError> {
    if output.is_closed() {
        return Err(RuntimeError::new(
            "display_closed",
            "terminal display is closed",
        ));
    }
    if let SessionCommand::Cancel {
        session_id,
        task_id,
    } = command
    {
        let value: Value = client
            .call(
                "task.cancel",
                json!({"session_id":session_id,"task_id":task_id}),
            )
            .await?;
        let status = value["status"]
            .as_str()
            .ok_or_else(|| RuntimeError::new("invalid_response", "cancel has no status"))?
            .into();
        if let Some(id) = request_id {
            let _ = output
                .send_reply(id, None, SessionEvent::Cancelled { status })
                .await;
        }
        return Ok(());
    }
    let mut streams = delivery.lock().await;
    if output.is_closed() {
        return Err(RuntimeError::new(
            "display_closed",
            "terminal display is closed",
        ));
    }
    let event = match command {
        SessionCommand::Send {
            session_id,
            message,
        } => {
            let value:Value=client.call("task.send",json!({"session_id":session_id,"message":{"text":message.text,"images":message.images},"context":message.context})).await?;
            let task_id = value["task_id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or_else(|| RuntimeError::new("invalid_response", "accepted task has no ID"))?
                .into();
            let message_id = value["message_id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or_else(|| RuntimeError::new("invalid_response", "accepted input has no ID"))?
                .into();
            let association = Association {
                session_id: session_id.clone(),
                task_id,
                message_id,
            };
            streams.insert(session_id, association.clone());
            if let Some(id) = request_id {
                let _ = output
                    .send_reply(id, Some(association), SessionEvent::Accepted)
                    .await;
            }
            return Ok(());
        }
        SessionCommand::Create => {
            let connections: Value = client.call("connection.list", json!({})).await?;
            let params = if connections["connections"]
                .as_array()
                .is_some_and(|x| x.len() > 1)
            {
                json!({"storage_schema":2,"config":{}})
            } else {
                json!({})
            };
            let state: SessionState = client.call("session.create", params).await?;
            snapshot(client, &state.session_id, None, false).await?
        }
        SessionCommand::Open { session_id } => {
            let _: SessionState = client
                .call("session.load", json!({"session_id":session_id}))
                .await?;
            snapshot(
                client,
                &session_id,
                streams.get(&session_id).cloned(),
                false,
            )
            .await?
        }
        SessionCommand::List => {
            #[derive(serde::Deserialize)]
            struct List {
                sessions: Vec<SessionSummary>,
            }
            SessionEvent::Listed(
                client
                    .call::<List>("session.list", json!({}))
                    .await?
                    .sessions,
            )
        }
        SessionCommand::Rename { session_id, title } => {
            #[derive(serde::Deserialize)]
            struct Renamed {
                metadata: SessionMetadata,
            }
            SessionEvent::Renamed(
                client
                    .call::<Renamed>(
                        "session.rename",
                        json!({"session_id":session_id,"title":title}),
                    )
                    .await?
                    .metadata,
            )
        }
        SessionCommand::Cancel { .. } => unreachable!(),
    };
    let result = match request_id {
        Some(id) => output.send_reply(id, None, event).await,
        None => output.send(event).await,
    };
    result.map_err(|e| RuntimeError::new("display_closed", e.to_string()))
}
async fn forward_events(
    client: Client,
    mut events: mpsc::UnboundedReceiver<crate::client::Event>,
    output: SessionEventSender,
    delivery: Delivery,
) -> Result<()> {
    while let Some(frame) = events.recv().await {
        let mut streams = delivery.lock().await;
        if frame.event["type"] == "session_metadata_updated" {
            #[derive(serde::Deserialize)]
            struct List {
                sessions: Vec<SessionSummary>,
            }
            if !output.is_closed() {
                match client.call::<List>("session.list", json!({})).await {
                    Ok(list) => {
                        for item in list
                            .sessions
                            .into_iter()
                            .filter(|item| item.metadata.id == frame.session_id)
                        {
                            let _ = output.send(SessionEvent::TitleUpdated(item.metadata)).await;
                        }
                    }
                    Err(error) => {
                        let _ = output.send(error_event("metadata", error)).await;
                    }
                }
            }
            continue;
        }
        let association = Association {
            session_id: frame.session_id.clone(),
            task_id: frame.task_id.context("task event has no task ID")?,
            message_id: frame
                .message_id
                .context("task event has no input message ID")?,
        };
        if frame.event["type"] == "task_finished" {
            let finished: TaskFinished = serde_json::from_value(frame.event)?;
            if finished.history_required && !output.is_closed() {
                match snapshot(&client, &frame.session_id, None, true).await {
                    Ok(event) => {
                        let _ = output.send_associated(association.clone(), event).await;
                    }
                    Err(error) => {
                        let _ = output
                            .send_associated(association.clone(), error_event("history", error))
                            .await;
                    }
                }
            }
            let _ = output
                .send_associated(association, SessionEvent::Finished(finished))
                .await;
            streams.remove(&frame.session_id);
        } else {
            let mut value = frame.event;
            if value["type"] == "task_started" {
                value["task_id"] = json!(association.task_id);
                value["message_id"] = json!(association.message_id);
            }
            let event: AgentEvent = serde_json::from_value(value)?;
            let _ = output
                .send_associated(
                    association,
                    SessionEvent::Agent {
                        session_id: frame.session_id,
                        event,
                    },
                )
                .await;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/session/terminal_delivery.rs"]
mod tests;
