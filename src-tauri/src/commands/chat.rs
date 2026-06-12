use std::sync::Arc;

use tauri::ipc::Channel;
use tauri::State;

use crate::agent::events::AgentEvent;
use crate::agent::loop_::{run_agent_loop, AgentCtx};
use crate::commands::mcp::McpState;
use crate::llm::registry;
use crate::llm::types::{ChatMessage, HistoryItem};
use crate::tools::mcp_adapter::McpToolAdapter;
use crate::tools::registry::ToolRegistry;
use crate::AppState;

#[tauri::command]
pub async fn send_message(
    provider: String,
    model: String,
    messages: Vec<ChatMessage>,
    channel: Channel<AgentEvent>,
    state: State<'_, AppState>,
    mcp: State<'_, McpState>,
) -> Result<(), String> {
    let endpoint = registry::resolve(&provider)?;
    let api_key = registry::api_key_for(&endpoint)?;
    let workspace = state.workspace.lock().unwrap().clone();
    let permissions = state.permissions.clone();

    // 内置工具 + 已连接 MCP server 的工具，组装本次请求的注册表
    let tool_registry = {
        let mut tools = state.tools.all();
        for connection in mcp.manager.lock().unwrap().connections() {
            tools.extend(McpToolAdapter::wrap_all(&connection));
        }
        Arc::new(ToolRegistry::from_tools(tools))
    };

    let history: Vec<HistoryItem> = truncate_history(
        messages
            .into_iter()
            .filter(|m| !m.content.is_empty())
            .map(|m| match m.role.as_str() {
                "assistant" => HistoryItem::Assistant { text: m.content, tool_calls: vec![] },
                _ => HistoryItem::User(m.content),
            })
            .collect(),
    );

    let on_event = |event: AgentEvent| {
        let _ = channel.send(event);
    };

    let ctx = AgentCtx {
        endpoint,
        api_key,
        model,
        registry: tool_registry,
        workspace,
        permissions,
    };
    let result = run_agent_loop(&ctx, history, &on_event).await;

    if let Err(message) = &result {
        on_event(AgentEvent::Error { message: message.clone() });
    }
    result
}

/// 超长对话截断：从最旧的消息开始丢，至少保留最后一条。
/// 粗略按字符数对齐上下文窗口（中文 1 字符 ≈ 1 token+，150K 字符对 256K 窗口留足余量）。
fn truncate_history(mut history: Vec<HistoryItem>) -> Vec<HistoryItem> {
    const MAX_CHARS: usize = 150_000;
    let size = |item: &HistoryItem| match item {
        HistoryItem::User(t) => t.chars().count(),
        HistoryItem::Assistant { text, .. } => text.chars().count(),
        HistoryItem::ToolResult { content, .. } => content.chars().count(),
    };
    let mut total: usize = history.iter().map(size).sum();
    while history.len() > 1 && total > MAX_CHARS {
        total -= size(&history.remove(0));
    }
    history
}

/// 前端对 PermissionAsk 的决议
#[tauri::command]
pub fn approve_permission(
    request_id: String,
    approved: bool,
    allow_all: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.permissions.resolve(&request_id, approved, allow_all)
}
