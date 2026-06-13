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
        duration_ms: u64,
        /// 该工具改动了文件时附带的检查点 id（前端据此显示"回滚"按钮）
        checkpoint_id: Option<String>,
    },
    /// 副作用操作审批请求：前端弹 diff/命令，用户经 approve_permission 决议
    #[serde(rename_all = "camelCase")]
    PermissionAsk {
        request_id: String,
        tool_name: String,
        /// 写文件 = 路径；执行命令 = 命令本身
        summary: String,
        diff: String,
        /// 命中危险模式时的警告原因（前端红色高亮）
        danger: Option<String>,
    },
    /// bash 命令的实时输出片段（流向前端终端面板）
    #[serde(rename_all = "camelCase")]
    CommandOutput { id: String, chunk: String },
    #[serde(rename_all = "camelCase")]
    TurnEnd {
        stop_reason: Option<String>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    },
    /// 上下文压缩发生时的提示（前端显示为系统注记）
    #[serde(rename_all = "camelCase")]
    ContextCompacted { note: String },
    #[serde(rename_all = "camelCase")]
    Error { message: String },
}
