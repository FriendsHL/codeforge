use serde::{Deserialize, Serialize};
use serde_json::Value;

/// LLM 调用的类型化错误：上层据此区分"用户取消 / 网络 / 鉴权 / 限流 / 服务端…"，
/// 不再靠字符串匹配。每个变体都能产出给用户看的友好中文。
#[derive(Debug, Clone)]
pub enum LlmError {
    /// 用户主动停止
    Cancelled,
    /// 连接失败 / 超时（网络、代理、服务不可达）
    Network(String),
    /// 401/403 鉴权
    Auth(String),
    /// 429 限流 / 配额
    RateLimit(String),
    /// 5xx 服务端
    Server(String),
    /// 其他 HTTP 错误
    Api(String),
    /// 响应解析 / 流读取失败
    Parse(String),
}

impl LlmError {
    pub fn is_cancelled(&self) -> bool {
        matches!(self, LlmError::Cancelled)
    }

    /// 给用户看的友好中文提示
    pub fn user_message(&self) -> String {
        match self {
            LlmError::Cancelled => "已停止".into(),
            LlmError::Network(d) => format!("无法连接到模型服务：请检查网络/代理，或确认服务地址可达（{d}）"),
            LlmError::Auth(d) => format!("API key 无效或无权限：请在设置中检查密钥。详情：{d}"),
            LlmError::RateLimit(d) => format!("请求过于频繁或配额耗尽：请稍后再试或更换模型。详情：{d}"),
            LlmError::Server(d) => format!("模型服务端错误：通常稍后重试即可。详情：{d}"),
            LlmError::Api(d) => format!("API 错误：{d}"),
            LlmError::Parse(d) => format!("响应解析失败：{d}"),
        }
    }

    pub fn from_send(e: &reqwest::Error) -> Self {
        if e.is_timeout() {
            LlmError::Network(format!("超时：{e}"))
        } else if e.is_connect() {
            LlmError::Network(format!("连接失败：{e}"))
        } else {
            LlmError::Network(e.to_string())
        }
    }

    pub fn from_status(status: u16, raw_message: &str) -> Self {
        let m = format!("HTTP {status}: {raw_message}");
        match status {
            401 | 403 => LlmError::Auth(m),
            429 => LlmError::RateLimit(m),
            500..=599 => LlmError::Server(m),
            _ => LlmError::Api(m),
        }
    }
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.user_message())
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
    /// 命中 prompt 缓存、按折扣计费的输入 token（Anthropic cache_read_input_tokens
    /// / OpenAI prompt_tokens_details.cached_tokens）。None=该 provider 未返回。
    pub cache_read_tokens: Option<u64>,
}

/// 流式过程中的增量事件
#[derive(Debug, Clone)]
pub enum LlmDelta {
    Text(String),
    /// 推理模型的思考过程（doubao / mimo 等会先输出 reasoning_content）
    Reasoning(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_maps_to_typed_error() {
        assert!(matches!(LlmError::from_status(401, "x"), LlmError::Auth(_)));
        assert!(matches!(LlmError::from_status(403, "x"), LlmError::Auth(_)));
        assert!(matches!(LlmError::from_status(429, "x"), LlmError::RateLimit(_)));
        assert!(matches!(LlmError::from_status(503, "x"), LlmError::Server(_)));
        assert!(matches!(LlmError::from_status(400, "x"), LlmError::Api(_)));
    }

    #[test]
    fn cancelled_is_distinguishable() {
        assert!(LlmError::Cancelled.is_cancelled());
        assert!(!LlmError::Auth("x".into()).is_cancelled());
    }

    #[test]
    fn user_message_is_friendly() {
        assert!(LlmError::Auth("HTTP 401".into()).user_message().contains("API key"));
        assert!(LlmError::RateLimit("x".into()).user_message().contains("频繁"));
        assert!(LlmError::Network("timeout".into()).user_message().contains("网络"));
    }
}
