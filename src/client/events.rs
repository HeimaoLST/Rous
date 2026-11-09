use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelRequestPurpose {
    Generation,
    Compaction,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    ContextUpdated {
        model: String,
        context_tokens: Option<usize>,
        context_window: Option<usize>,
        input_limit: Option<usize>,
    },
    TaskStarted {
        task_id: String,
        message_id: Option<String>,
    },
    ModelRequestStarted {
        purpose: ModelRequestPurpose,
    },
    ResponseText {
        item_id: String,
        text: String,
    },
    ResponseTextDelta {
        item_id: String,
        delta: String,
    },
    Reasoning {
        item_id: String,
        text: String,
    },
    ReasoningDelta {
        item_id: String,
        delta: String,
    },
    ReasoningCompleted {
        item_id: String,
    },
    ToolStarted {
        call_id: String,
        name: String,
        arguments: String,
    },
    ToolFinished {
        call_id: String,
        name: String,
        output: Value,
    },
    Compaction {
        tokens_before: usize,
        tokens_after: usize,
        summary_output_tokens: Option<u64>,
    },
    Usage {
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    },
    TurnCompleted,
    TurnInterrupted,
    TurnFailed {
        message: String,
    },
}
