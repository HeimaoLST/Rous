use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Developer,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InputMessageContent {
    Text(String),
    Parts(Vec<InputContentPart>),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputContentPart {
    Text { text: String },
    Image { url: String },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningText {
    pub text: String,
}
#[cfg(test)]
impl ReasoningText {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputItem {
    Message {
        role: MessageRole,
        content: InputMessageContent,
    },
    ToolCall {
        call_id: String,
        name: String,
        arguments: String,
    },
    ToolResult {
        call_id: String,
        output: String,
    },
    Reasoning {
        content: Vec<ReasoningText>,
        provider_data: Option<Value>,
    },
    ProviderItem {
        data: Value,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    pub title_source: Option<String>,
    pub created_at: u64,
    pub working_directory: PathBuf,
    pub model: String,
    pub api: String,
    pub compaction: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveTaskState {
    pub task_id: String,
    pub message_id: String,
    pub cancel_requested: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub session_id: String,
    pub metadata: Option<SessionMetadata>,
    pub loaded: bool,
    pub state: String,
    pub unavailable: Option<Value>,
    pub active_task: Option<ActiveTaskState>,
    pub status: String,
    pub effective_config: Option<Value>,
    pub config_revision: Option<u64>,
    pub storage_schema: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub metadata: SessionMetadata,
    pub updated_at: u64,
    pub status: super::history::TaskStatus,
    pub in_use: bool,
    pub unavailable: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Association {
    pub session_id: String,
    pub task_id: String,
    pub message_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeError {
    pub code: String,
    pub message: String,
    pub session_id: Option<String>,
    pub reason: Option<String>,
    pub current_revision: Option<u64>,
}
impl RuntimeError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            session_id: None,
            reason: None,
            current_revision: None,
        }
    }
}
impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for RuntimeError {}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskFinished {
    pub status: String,
    pub error: Option<RuntimeError>,
    pub persistence: Option<String>,
    #[serde(default)]
    pub history_required: bool,
}
#[derive(Debug, Deserialize)]
pub struct ContextInfo {
    pub model: String,
    pub context_tokens: Option<usize>,
    pub context_window: Option<usize>,
    pub input_limit: Option<usize>,
}
#[derive(Debug, Deserialize)]
pub struct Snapshot {
    pub session_id: String,
    pub metadata: SessionMetadata,
    pub session: SessionState,
    pub records: Vec<super::history::HistoryRecord>,
    pub context: ContextInfo,
}
