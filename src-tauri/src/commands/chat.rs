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
    session_id: Option<i64>,
    channel: Channel<AgentEvent>,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    mcp: State<'_, McpState>,
) -> Result<(), String> {
    let endpoint = registry::resolve(&provider)?;
    let api_key = registry::api_key_for(&endpoint)?;
    let workspace = state.workspace.lock().unwrap().clone();
    let permissions = state.permissions.clone();

    // 本回合的任务清单（todo_write 维护，loop 注入为 system-reminder）
    let todos: crate::tools::todo::TodoList = Arc::new(std::sync::Mutex::new(Vec::new()));

    // 内置工具 + browser_open / todo_write（需要 AppHandle/状态）+ 已连接 MCP server 的工具
    let tool_registry = {
        let mut tools = state.tools.all();
        tools.push(Arc::new(crate::tools::browser::BrowserOpenTool { app: app.clone() })
            as Arc<dyn crate::tools::registry::Tool>);
        tools.push(Arc::new(crate::tools::todo::TodoWriteTool {
            todos: todos.clone(),
            app: app.clone(),
        }) as Arc<dyn crate::tools::registry::Tool>);
        for connection in mcp.manager.lock().unwrap().connections() {
            tools.extend(McpToolAdapter::wrap_all(&connection));
        }
        Arc::new(ToolRegistry::from_tools(tools))
    };

    let raw_history: Vec<HistoryItem> = messages
        .into_iter()
        .filter(|m| !m.content.is_empty())
        .map(|m| match m.role.as_str() {
            "assistant" => HistoryItem::Assistant { text: m.content, tool_calls: vec![] },
            _ => HistoryItem::User(m.content),
        })
        .collect();

    let on_event = |event: AgentEvent| {
        let _ = channel.send(event);
    };

    // 跨轮压缩：超过阈值时把早前对话交给便宜模型摘要；失败则退回硬截断
    let history = match compact_history(&endpoint, &api_key, &provider, &state.cancel, raw_history).await {
        (history, Some(note)) => {
            on_event(AgentEvent::ContextCompacted { note });
            history
        }
        (history, None) => history,
    };

    state.cancel.store(false, std::sync::atomic::Ordering::SeqCst); // 新回合清掉旧的停止标志

    // OTel 风格本地 trace：traces/<session_id>.jsonl（无会话时不记）
    let trace = session_id.and_then(|sid| {
        use tauri::Manager;
        app.path()
            .app_data_dir()
            .ok()
            .and_then(|dir| crate::trace::TraceWriter::open(&dir.join("traces"), sid).ok())
            .map(Arc::new)
    });

    let ctx = AgentCtx {
        endpoint,
        api_key,
        model,
        registry: tool_registry,
        workspace,
        permissions,
        cancel: state.cancel.clone(),
        provider: provider.clone(),
        trace,
        todos,
    };
    let result = run_agent_loop(&ctx, history, &on_event).await;

    if let Err(message) = &result {
        on_event(AgentEvent::Error { message: message.clone() });
    }
    result
}

fn item_chars(item: &HistoryItem) -> usize {
    match item {
        HistoryItem::User(t) => t.chars().count(),
        HistoryItem::Assistant { text, .. } => text.chars().count(),
        HistoryItem::ToolResult { content, .. } => content.chars().count(),
    }
}

/// 兜底硬截断：从最旧的消息开始丢，至少保留最后一条
fn truncate_history(mut history: Vec<HistoryItem>) -> Vec<HistoryItem> {
    const MAX_CHARS: usize = 150_000;
    let mut total: usize = history.iter().map(item_chars).sum();
    while history.len() > 1 && total > MAX_CHARS {
        total -= item_chars(&history.remove(0));
    }
    history
}

/// 摘要压缩用的便宜模型（同 provider，省钱且 key 现成）
fn cheap_model_for(provider: &str) -> &'static str {
    match provider {
        "ark" => "doubao-seed-2.0-lite",
        "xiaomi-mimo" => "mimo-v2.5",
        _ => "claude-haiku-4-5",
    }
}

const COMPACT_TRIGGER_CHARS: usize = 100_000;
const KEEP_RECENT_MESSAGES: usize = 10; // 保留最近 ~5 轮（user+assistant）的原文

/// 超阈值时：保留最近 N 条消息原文，更早的全部交给便宜模型做三段式摘要。
/// 返回 (新历史, 提示文案)
async fn compact_history(
    endpoint: &registry::Endpoint,
    api_key: &str,
    provider: &str,
    cancel: &Arc<std::sync::atomic::AtomicBool>,
    history: Vec<HistoryItem>,
) -> (Vec<HistoryItem>, Option<String>) {
    let total: usize = history.iter().map(item_chars).sum();
    if total <= COMPACT_TRIGGER_CHARS {
        return (history, None);
    }
    // 太短不值得摘要（保不住最近 N 条就直接硬截断兜底）
    if history.len() <= KEEP_RECENT_MESSAGES + 1 {
        return (truncate_history(history), Some("对话过长，已截断最早的消息".into()));
    }

    let split = history.len() - KEEP_RECENT_MESSAGES;
    let old = &history[..split];
    let old_chars: usize = old.iter().map(item_chars).sum();
    let transcript: String = old
        .iter()
        .map(|item| match item {
            HistoryItem::User(t) => format!("用户: {t}"),
            HistoryItem::Assistant { text, .. } => format!("助手: {text}"),
            HistoryItem::ToolResult { name, .. } => format!("(工具 {name} 的结果，略)"),
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let system = "你在压缩一段 coding agent 的对话历史，供后续轮次接续使用。严格输出以下三个小节（无内容写「无」），用紧凑中文要点，不要寒暄/评论：\n## 已确认的决定\n（用户目标与约束、已敲定的方案、关键文件路径与代码标识符）\n## 当前状态\n（已完成的改动、已验证的结论、当前所处步骤）\n## 待办事项\n（未完成的任务、下一步计划、遗留问题）";
    let summary_history = vec![HistoryItem::User(transcript)];
    let cheap = cheap_model_for(provider);
    let result = match endpoint {
        registry::Endpoint::Anthropic => {
            crate::llm::anthropic::stream_chat(api_key, cheap, Some(system), &summary_history, &[], cancel, |_| {}).await
        }
        registry::Endpoint::OpenAiCompatible { chat_url, .. } => {
            crate::llm::openai::stream_chat(chat_url, api_key, cheap, Some(system), &summary_history, &[], cancel, |_| {}).await
        }
    };

    match result {
        Ok(turn) if !turn.text.trim().is_empty() => {
            let summary_chars = turn.text.chars().count();
            let mut compacted = vec![HistoryItem::User(format!(
                "[早前对话的自动摘要，原文已压缩；最近 {KEEP_RECENT_MESSAGES} 条消息保留在后面]\n{}",
                turn.text.trim()
            ))];
            compacted.extend_from_slice(&history[split..]);
            (
                compacted,
                Some(format!(
                    "已把早前 {split} 条消息压缩为三段式摘要（{old_chars} → {summary_chars} 字符），保留最近 {KEEP_RECENT_MESSAGES} 条原文"
                )),
            )
        }
        _ => (
            truncate_history(history),
            Some("摘要压缩失败，已按旧策略截断最早消息".into()),
        ),
    }
}

/// 停止当前回合：流式读取/loop/子 agent 尽快收尾；挂起的审批按拒绝处理
#[tauri::command]
pub fn stop_generation(state: State<'_, AppState>) -> Result<(), String> {
    state.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    state.permissions.deny_all_pending();
    Ok(())
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
