pub mod events;
pub mod history;
pub mod types;
pub use events::AgentEvent;
#[cfg(test)]
pub use events::ModelRequestPurpose;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::{mpsc, oneshot, watch},
};
pub use types::{
    Association, RuntimeError, SessionMetadata, SessionState, SessionSummary, TaskFinished,
};
const MAX_FRAME: usize = 8 * 1024 * 1024;
#[derive(Clone, Copy)]
struct Timeouts {
    startup: Duration,
    write: Duration,
    response: Duration,
}
impl Default for Timeouts {
    fn default() -> Self {
        Self {
            startup: Duration::from_secs(45),
            write: Duration::from_secs(30),
            response: Duration::from_secs(30),
        }
    }
}
type Reply = Result<Value, RuntimeError>;
type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Reply>>>>;
#[derive(Debug)]
pub struct Event {
    pub session_id: String,
    pub task_id: Option<String>,
    pub message_id: Option<String>,
    pub event: Value,
    _bytes: tokio::sync::OwnedSemaphorePermit,
}
#[derive(Clone)]
pub struct Client {
    input: Arc<tokio::sync::Mutex<Option<ChildStdin>>>,
    pending: Pending,
    next: Arc<AtomicU64>,
    closed: watch::Sender<bool>,
    timeouts: Timeouts,
}
pub struct RuntimeProcess {
    pub client: Client,
    pub events: mpsc::UnboundedReceiver<Event>,
    child: Child,
    reader: tokio::task::JoinHandle<anyhow::Result<()>>,
    stderr: tokio::task::JoinHandle<()>,
}
fn fail_pending(pending: &Pending, message: &str) {
    for (_, tx) in pending.lock().unwrap().drain() {
        let _ = tx.send(Err(RuntimeError::new("disconnected", message)));
    }
}
impl Client {
    pub async fn call<T: DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<T, RuntimeError> {
        if *self.closed.borrow() {
            return Err(RuntimeError::new(
                "disconnected",
                "Runtime output is closed",
            ));
        }
        let id = format!("r{}", self.next.fetch_add(1, Ordering::Relaxed));
        let mut bytes =
            serde_json::to_vec(&json!({"version":1,"id":id,"method":method,"params":params}))
                .map_err(|e| RuntimeError::new("invalid_request", e.to_string()))?;
        if bytes.len() > MAX_FRAME {
            return Err(RuntimeError::new(
                "invalid_request",
                "request exceeds 8 MiB",
            ));
        }
        bytes.push(b'\n');
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id.clone(), tx);
        let result = async {
            let mut input = self.input.lock().await;
            if *self.closed.borrow() {
                return Err(RuntimeError::new("disconnected", "Runtime input is closed"));
            }
            let writer = input
                .as_mut()
                .ok_or_else(|| RuntimeError::new("disconnected", "Runtime input is closed"))?;
            match tokio::time::timeout(self.timeouts.write, writer.write_all(&bytes)).await {
                Ok(Ok(())) => {}
                result => {
                    // A cancelled/failed write may have emitted a partial JSON frame.
                    input.take();
                    self.closed.send_replace(true);
                    fail_pending(
                        &self.pending,
                        "Runtime input closed after an uncertain partial write",
                    );
                    return Err(RuntimeError::new(
                        "delivery_unknown",
                        match result {
                            Ok(Err(error)) => error.to_string(),
                            _ => "Runtime write timed out; input closed".into(),
                        },
                    ));
                }
            }
            drop(input);
            let value = tokio::time::timeout(self.timeouts.response, rx)
                .await
                .map_err(|_| {
                    RuntimeError::new(
                        "delivery_unknown",
                        "Runtime response timed out; inspect state before retrying",
                    )
                })?
                .map_err(|_| {
                    RuntimeError::new("disconnected", "Runtime closed before replying")
                })??;
            serde_json::from_value(value)
                .map_err(|e| RuntimeError::new("invalid_response", e.to_string()))
        }
        .await;
        self.pending.lock().unwrap().remove(&id);
        result
    }
}
impl RuntimeProcess {
    pub async fn start(command: Command) -> anyhow::Result<Self> {
        Self::start_with_options(command, Timeouts::default()).await
    }
    async fn start_with_options(mut command: Command, timeouts: Timeouts) -> anyhow::Result<Self> {
        let mut child = command
            .arg("--stdio")
            .env("SAKI_AUTOMATIC_TITLES", "true")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let input = Arc::new(tokio::sync::Mutex::new(child.stdin.take()));
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let pending: Pending = Default::default();
        // Event count is bounded by bytes rather than an arbitrary small burst limit.
        let (event_tx, events) = mpsc::unbounded_channel();
        let (closed, _) = watch::channel(false);
        let (ready_tx, ready) = oneshot::channel();
        let replies = pending.clone();
        let done = closed.clone();
        let writer = input.clone();
        let reader = tokio::spawn(async move {
            let result = read_output(stdout, replies.clone(), event_tx, ready_tx).await;
            done.send_replace(true);
            fail_pending(&replies, "Runtime output closed; delivery may be unknown");
            writer.lock().await.take();
            result
        });
        let stderr = tokio::spawn(async move {
            let mut input = BufReader::new(stderr);
            let mut bytes = Vec::new();
            loop {
                bytes.clear();
                match (&mut input).take(8192).read_until(b'\n', &mut bytes).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => eprint!("{}", String::from_utf8_lossy(&bytes)),
                }
            }
        });
        let mut process = Self {
            client: Client {
                input,
                pending,
                next: Arc::new(AtomicU64::new(1)),
                closed,
                timeouts,
            },
            events,
            child,
            reader,
            stderr,
        };
        let started = tokio::time::timeout(timeouts.startup, ready).await;
        if matches!(started, Ok(Ok(()))) {
            return Ok(process);
        }
        process.client.closed.send_replace(true);
        process.client.input.lock().await.take();
        let _ = process.child.kill().await;
        let _ = process.child.wait().await;
        let reader = process.reader.await;
        let _ = process.stderr.await;
        match started {
            Err(_) => anyhow::bail!("Runtime initialization exceeded {:?}", timeouts.startup),
            _ => {
                if let Ok(Err(error)) = reader {
                    return Err(error.context("Runtime failed before ready"));
                }
                anyhow::bail!("Runtime exited before ready")
            }
        }
    }
    pub async fn finish(mut self) -> anyhow::Result<()> {
        self.client.input.lock().await.take();
        while self.events.recv().await.is_some() {}
        let status = self.child.wait().await?;
        self.reader.await??;
        self.stderr.await?;
        anyhow::ensure!(status.success(), "Runtime exited with {status}");
        Ok(())
    }
}
async fn read_output(
    stdout: tokio::process::ChildStdout,
    pending: Pending,
    events: mpsc::UnboundedSender<Event>,
    ready: oneshot::Sender<()>,
) -> anyhow::Result<()> {
    let mut input = BufReader::new(stdout);
    let mut bytes = Vec::new();
    let mut ready = Some(ready);
    let budget = Arc::new(tokio::sync::Semaphore::new(MAX_FRAME));
    loop {
        bytes.clear();
        if (&mut input)
            .take((MAX_FRAME + 1) as u64)
            .read_until(b'\n', &mut bytes)
            .await?
            == 0
        {
            break;
        }
        anyhow::ensure!(bytes.len() <= MAX_FRAME, "Runtime frame exceeds 8 MiB");
        let mut value: Value = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(
            value["version"] == 1,
            "unsupported Runtime protocol version"
        );
        match value["type"].as_str() {
            Some("ready") => {
                if let Some(tx) = ready.take() {
                    let _ = tx.send(());
                }
            }
            Some("response") => {
                if let Some(id) = value["id"].as_str()
                    && let Some(tx) = pending.lock().unwrap().remove(id)
                {
                    let reply = if value.get("error").is_some() {
                        Err(serde_json::from_value(value["error"].take())?)
                    } else {
                        Ok(value["result"].take())
                    };
                    let _ = tx.send(reply);
                }
            }
            Some("event") => {
                let event=Event{
                    session_id:value["session_id"].as_str().ok_or_else(||anyhow::anyhow!("event has no session_id"))?.into(),
                    task_id:value["task_id"].as_str().map(str::to_owned),message_id:value["message_id"].as_str().map(str::to_owned),
                    event:value["event"].take(),
                    _bytes:budget.clone().try_acquire_many_owned(bytes.len() as u32).map_err(|_|anyhow::anyhow!("Runtime event byte budget exhausted; inspect history before continuing"))?,
                };
                events
                    .send(event)
                    .map_err(|_| anyhow::anyhow!("Runtime observation closed"))?;
            }
            _ => anyhow::bail!("unknown Runtime frame kind"),
        }
    }
    Ok(())
}
#[cfg(test)]
pub(crate) mod tests;
