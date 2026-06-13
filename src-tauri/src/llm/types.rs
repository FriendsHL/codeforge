use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 用户主动停止的标记错误（loop 捕获后优雅收尾，不当成真错误）
pub const CANCELLED_ERR: &str = "__CF_CANCELLED__";

/// 把 reqwest 发送错误转成给用户看的友好中文提示
pub fn friendly_send_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "请求超时：模型服务长时间无响应，请稍后重试或检查网络/代理".into()
    } else if e.is_connect() {
        "无法连接到模型服务：请检查网络连接、代理设置，或确认服务地址可达".into()
    } else {
        format!("请求失败：{e}")
    }
}

/// 把 HTTP 错误状态转成友好提示（401/403=鉴权，429=限流，5xx=服务端）
pub fn friendly_status_error(status: u16, raw_message: &str) -> String {
    match status {
        401 | 403 => format!("API key 无效或无权限（HTTP {status}）：请在设置中检查密钥。详情：{raw_message}"),
        429 => format!("请求过于频繁或配额耗尽（HTTP 429）：请稍后再试或更换模型。详情：{raw_message}"),
        500..=599 => format!("模型服务端错误（HTTP {status}）：通常稍后重试即可。详情：{raw_message}"),
        _ => format!("API 错误（HTTP {status}）：{raw_message}"),
    }
}

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
    /// 本次请求的真实上下文大小（API usage 返回）
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

/// 流式过程中的增量事件
#[derive(Debug, Clone)]
pub enum LlmDelta {
    Text(String),
    /// 推理模型的思考过程（doubao / mimo 等会先输出 reasoning_content）
    Reasoning(String),
}
