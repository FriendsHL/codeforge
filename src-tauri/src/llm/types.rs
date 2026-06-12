use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 用户主动停止的标记错误（loop 捕获后优雅收尾，不当成真错误）
pub const CANCELLED_ERR: &str = "__CF_CANCELLED__";

/// 前端发来的简单消息（不含工具轮次）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// 模型发起的一次工具调用（arguments 为 JSON 字符串）
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// provider 无关的对话历史项，由各 provider 转成自己的 wire 格式
#[derive(Debug, Clone)]
pub enum HistoryItem {
    User(String),
    Assistant {
        text: String,
        tool_calls: Vec<ToolCall>,
    },
    ToolResult {
        call_id: String,
        /// 工具名暂未上 wire（两家协议都只认 call_id），留作 M5 持久化用
        #[allow(dead_code)]
        name: String,
        content: String,
        is_error: bool,
    },
}

/// 工具的对外声明（注册表产出，喂给 LLM）
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// 一次模型调用的完整结果（流式 delta 通过回调旁路输出）
#[derive(Debug, Default)]
pub struct AssistantTurn {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub stop_reason: Option<String>,
    pub output_tokens: Option<u64>,
}

/// 流式过程中的增量事件
#[derive(Debug, Clone)]
pub enum LlmDelta {
    Text(String),
    /// 推理模型的思考过程（doubao / mimo 等会先输出 reasoning_content）
    Reasoning(String),
}
