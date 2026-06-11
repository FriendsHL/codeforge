//! agent 核心循环：调 LLM → 解析工具调用 → 执行 → 结果回填 → 再调 LLM，直到无工具调用

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

use super::events::AgentEvent;
use super::prompt;
use crate::llm::registry::Endpoint;
use crate::llm::types::{AssistantTurn, HistoryItem, LlmDelta, ToolSpec};
use crate::llm::{anthropic, openai};
use crate::tools::registry::ToolRegistry;

const MAX_ITERATIONS: usize = 30;
/// 工具结果回传前端展示时的截断长度（回填给模型的是全量）
const EVENT_OUTPUT_PREVIEW_CHARS: usize = 2000;

pub async fn run_agent_loop(
    endpoint: &Endpoint,
    api_key: &str,
    model: &str,
    mut history: Vec<HistoryItem>,
    registry: Arc<ToolRegistry>,
    workspace: Option<PathBuf>,
    on_event: impl Fn(AgentEvent),
) -> Result<(), String> {
    let system = prompt::build_system_prompt(workspace.as_deref());
    let tools: Vec<ToolSpec> = if workspace.is_some() {
        registry.specs()
    } else {
        Vec::new()
    };

    for _ in 0..MAX_ITERATIONS {
        let turn = call_llm(endpoint, api_key, model, &system, &history, &tools, &on_event).await?;

        let stop_reason = turn.stop_reason.clone();
        let output_tokens = turn.output_tokens;
        let tool_calls = turn.tool_calls.clone();

        history.push(HistoryItem::Assistant {
            text: turn.text,
            tool_calls: tool_calls.clone(),
        });

        if tool_calls.is_empty() {
            on_event(AgentEvent::TurnEnd { stop_reason, output_tokens });
            return Ok(());
        }

        for call in tool_calls {
            let input: Value = serde_json::from_str(&call.arguments)
                .unwrap_or(Value::Object(Default::default()));
            on_event(AgentEvent::ToolCallStart {
                id: call.id.clone(),
                name: call.name.clone(),
                input: input.clone(),
            });

            let result = execute_tool(&registry, workspace.as_ref(), &call.name, input).await;
            let (content, is_error) = match result {
                Ok(content) => (content, false),
                Err(message) => (message, true),
            };

            on_event(AgentEvent::ToolCallEnd {
                id: call.id.clone(),
                output: preview(&content),
                is_error,
            });
            history.push(HistoryItem::ToolResult {
                call_id: call.id,
                name: call.name,
                content,
                is_error,
            });
        }
    }

    Err(format!("达到最大迭代次数（{MAX_ITERATIONS}），任务可能过于复杂，请拆小后重试"))
}

async fn call_llm(
    endpoint: &Endpoint,
    api_key: &str,
    model: &str,
    system: &str,
    history: &[HistoryItem],
    tools: &[ToolSpec],
    on_event: &impl Fn(AgentEvent),
) -> Result<AssistantTurn, String> {
    let on_delta = |delta: LlmDelta| match delta {
        LlmDelta::Text(text) => on_event(AgentEvent::TextDelta { text }),
        LlmDelta::Reasoning(text) => on_event(AgentEvent::ReasoningDelta { text }),
    };
    match endpoint {
        Endpoint::Anthropic => {
            anthropic::stream_chat(api_key, model, Some(system), history, tools, on_delta).await
        }
        Endpoint::OpenAiCompatible { chat_url, .. } => {
            openai::stream_chat(chat_url, api_key, model, Some(system), history, tools, on_delta)
                .await
        }
    }
}

async fn execute_tool(
    registry: &Arc<ToolRegistry>,
    workspace: Option<&PathBuf>,
    name: &str,
    input: Value,
) -> Result<String, String> {
    let Some(workspace) = workspace.cloned() else {
        return Err("未打开工作区，无法使用工具".into());
    };
    let Some(tool) = registry.get(name) else {
        return Err(format!("未知工具: {name}"));
    };
    // 文件 IO 放到阻塞线程池，避免卡住异步运行时
    tokio::task::spawn_blocking(move || tool.run(&workspace, &input))
        .await
        .map_err(|e| format!("工具执行崩溃: {e}"))?
}

fn preview(content: &str) -> String {
    if content.chars().count() <= EVENT_OUTPUT_PREVIEW_CHARS {
        content.to_string()
    } else {
        let shown: String = content.chars().take(EVENT_OUTPUT_PREVIEW_CHARS).collect();
        format!("{shown}\n…[界面预览截断，模型收到的是完整结果]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::registry;

    /// 真实 API 集成测试：需要 ARK_API_KEY，手动运行
    /// cargo test live_agent_loop -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_agent_loop_reads_own_codebase() {
        let endpoint = registry::resolve("ark").unwrap();
        let api_key = registry::api_key_for(&endpoint).unwrap();
        // src-tauri 的上级目录 = codeforge 项目根
        let workspace = std::env::current_dir().unwrap().parent().unwrap().to_path_buf();

        let history = vec![HistoryItem::User(
            "这个项目 Rust 端实现了哪几个 agent 工具？只列工具名。".into(),
        )];
        let tool_call_count = std::sync::atomic::AtomicUsize::new(0);
        let final_text = std::sync::Mutex::new(String::new());

        run_agent_loop(
            &endpoint,
            &api_key,
            "doubao-seed-2.0-pro",
            history,
            Arc::new(ToolRegistry::builtin()),
            Some(workspace),
            |event| match event {
                AgentEvent::ToolCallStart { name, input, .. } => {
                    tool_call_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    println!(">> 工具调用: {name} {input}");
                }
                AgentEvent::TextDelta { text } => {
                    final_text.lock().unwrap().push_str(&text);
                }
                AgentEvent::TurnEnd { .. } => println!("<< 回合结束"),
                _ => {}
            },
        )
        .await
        .unwrap();

        let text = final_text.lock().unwrap().clone();
        println!("最终回答:\n{text}");
        assert!(
            tool_call_count.load(std::sync::atomic::Ordering::SeqCst) > 0,
            "agent 应该至少调用一次工具"
        );
        assert!(
            text.contains("read_file") && text.contains("grep"),
            "回答应包含真实的工具名，实际: {text}"
        );
    }
}
