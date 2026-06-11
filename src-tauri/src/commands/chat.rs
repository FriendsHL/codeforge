use tauri::ipc::Channel;
use tauri::State;

use crate::agent::events::AgentEvent;
use crate::agent::loop_::run_agent_loop;
use crate::llm::registry;
use crate::llm::types::{ChatMessage, HistoryItem};
use crate::AppState;

#[tauri::command]
pub async fn send_message(
    provider: String,
    model: String,
    messages: Vec<ChatMessage>,
    channel: Channel<AgentEvent>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let endpoint = registry::resolve(&provider)?;
    let api_key = registry::api_key_for(&endpoint)?;
    let workspace = state.workspace.lock().unwrap().clone();
    let tool_registry = state.tools.clone();
    let permissions = state.permissions.clone();

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

    let result = run_agent_loop(
        &endpoint,
        &api_key,
        &model,
        history,
        tool_registry,
        workspace,
        permissions,
        &on_event,
    )
    .await;

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
