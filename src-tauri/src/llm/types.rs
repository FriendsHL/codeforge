use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// 流式过程中向上层回调的统一事件
#[derive(Debug, Clone)]
pub enum LlmEvent {
    TextDelta(String),
    /// 推理模型的思考过程（doubao / mimo 等会先输出 reasoning_content）
    ReasoningDelta(String),
    TurnEnd {
        stop_reason: Option<String>,
        output_tokens: Option<u64>,
    },
}
