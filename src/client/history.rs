use super::types::InputItem;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Completed,
    Failed,
    Interrupted,
    #[default]
    #[serde(other)]
    Unknown,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HistoryEntry {
    Messages {
        items: Vec<InputItem>,
    },
    Compaction {
        summary: String,
        tokens_before: usize,
        tokens_after: usize,
        usage: Option<Usage>,
    },
    TaskStarted {
        task_id: String,
        message_id: Option<String>,
        configuration: Option<Value>,
    },
    TaskFinished {
        task_id: String,
        status: TaskStatus,
        error: Option<String>,
    },
}
#[derive(Debug, Clone, Deserialize)]
pub struct HistoryRecord {
    pub record: HistoryEntry,
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct History {
    pub history: Vec<HistoryEntry>,
    pub status: TaskStatus,
}
impl History {
    pub fn has_task_history(&self) -> bool {
        self.history
            .iter()
            .any(|record| matches!(record, HistoryEntry::TaskStarted { .. }))
    }
}
#[cfg(test)]
pub fn new_id() -> anyhow::Result<String> {
    use std::{
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    Ok(format!(
        "{}-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

pub fn now_ms() -> anyhow::Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as u64)
}
