use serde::Serialize;
use serde_json::Value;

/// 推送给前端的统一事件协议
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AgentEvent {
    #[serde(rename_all = "camelCase")]
    TextDelta { text: String },
    #[serde(rename_all = "camelCase")]
    ReasoningDelta { text: String },
    #[serde(rename_all = "camelCase")]
    ToolCallStart { id: String, name: String, input: Value },
    #[serde(rename_all = "camelCase")]
    ToolCallEnd {
        id: String,
        output: String,
        is_error: bool,
    },
    #[serde(rename_all = "camelCase")]
    TurnEnd {
        stop_reason: Option<String>,
        output_tokens: Option<u64>,
    },
    #[serde(rename_all = "camelCase")]
    Error { message: String },
}
