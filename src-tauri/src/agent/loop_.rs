//! agent 核心循环：调 LLM → 解析工具调用 → 执行 → 结果回填 → 再调 LLM，直到无工具调用

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

use super::events::AgentEvent;
use super::prompt;
use crate::llm::registry::Endpoint;
use crate::llm::types::{AssistantTurn, HistoryItem, LlmDelta, ToolSpec};
use crate::llm::{anthropic, openai};
use crate::security::PermissionManager;
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
    permissions: Arc<PermissionManager>,
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

            let result = execute_tool(
                &registry,
                workspace.as_ref(),
                &permissions,
                &call.id,
                &call.name,
                input,
                &on_event,
            )
            .await;
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
    permissions: &Arc<PermissionManager>,
    call_id: &str,
    name: &str,
    input: Value,
    on_event: &impl Fn(AgentEvent),
) -> Result<String, String> {
    let Some(workspace) = workspace.cloned() else {
        return Err("未打开工作区，无法使用工具".into());
    };
    let Some(tool) = registry.get(name) else {
        return Err(format!("未知工具: {name}"));
    };

    // 写类工具先预演 diff 走审批
    let plan = {
        let tool = tool.clone();
        let workspace = workspace.clone();
        let input = input.clone();
        tokio::task::spawn_blocking(move || tool.plan(&workspace, &input))
            .await
            .map_err(|e| format!("工具执行崩溃: {e}"))??
    };
    if let Some(plan) = plan {
        if !permissions.is_allow_all() {
            // 先注册再发事件，避免决议先于等待到达的竞态
            let rx = permissions.register(call_id);
            on_event(AgentEvent::PermissionAsk {
                request_id: call_id.to_string(),
                tool_name: name.to_string(),
                summary: plan.summary,
                diff: plan.diff,
            });
            if !permissions.wait(call_id, rx).await {
                return Err("用户拒绝了本次操作。请询问用户的意图后再调整方案，不要原样重试。".into());
            }
        }
    }

    // 工具在阻塞线程池执行；流式输出经通道转回异步侧推给前端
    let (chunk_tx, mut chunk_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let handle = tokio::task::spawn_blocking(move || {
        let mut on_chunk = |chunk: &str| {
            let _ = chunk_tx.send(chunk.to_string());
        };
        tool.run_streaming(&workspace, &input, &mut on_chunk)
    });
    while let Some(chunk) = chunk_rx.recv().await {
        on_event(AgentEvent::CommandOutput { id: call_id.to_string(), chunk });
    }
    handle.await.map_err(|e| format!("工具执行崩溃: {e}"))?
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
            Arc::new(PermissionManager::default()),
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

    /// 真实 API 集成测试：agent 改文件 + 审批放行后真正落盘
    /// cargo test live_agent_edits -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_agent_edits_file_after_approval() {
        let endpoint = registry::resolve("ark").unwrap();
        let api_key = registry::api_key_for(&endpoint).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().canonicalize().unwrap();
        std::fs::write(workspace.join("notes.txt"), "hello codeforge\n").unwrap();

        let permissions = Arc::new(PermissionManager::default());
        let pm = permissions.clone();
        let asked = std::sync::atomic::AtomicBool::new(false);

        run_agent_loop(
            &endpoint,
            &api_key,
            "doubao-seed-2.0-pro",
            vec![HistoryItem::User(
                "把 notes.txt 里的 hello 改成 goodbye，其他内容不动。".into(),
            )],
            Arc::new(ToolRegistry::builtin()),
            Some(workspace.clone()),
            permissions,
            |event| match event {
                AgentEvent::PermissionAsk { request_id, summary, diff, .. } => {
                    println!(">> 审批请求: {summary}\n{diff}");
                    asked.store(true, std::sync::atomic::Ordering::SeqCst);
                    assert!(diff.contains("-hello codeforge"));
                    assert!(diff.contains("+goodbye codeforge"));
                    pm.resolve(&request_id, true, false).unwrap(); // 模拟用户点"允许"
                }
                AgentEvent::ToolCallStart { name, input, .. } => {
                    println!(">> 工具调用: {name} {input}");
                }
                _ => {}
            },
        )
        .await
        .unwrap();

        assert!(asked.load(std::sync::atomic::Ordering::SeqCst), "应弹出审批");
        let content = std::fs::read_to_string(workspace.join("notes.txt")).unwrap();
        assert_eq!(content, "goodbye codeforge\n", "审批通过后文件应已修改");
        println!("文件最终内容: {content}");
    }

    /// 真实 API 集成测试：自我纠错闭环（跑测试 → 失败 → 修代码 → 重跑直到通过）
    /// cargo test live_agent_self_corrects -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_agent_self_corrects_with_bash() {
        let endpoint = registry::resolve("ark").unwrap();
        let api_key = registry::api_key_for(&endpoint).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().canonicalize().unwrap();
        std::fs::write(workspace.join("calc.py"), "def add(a, b):\n    return a - b\n").unwrap();
        std::fs::write(
            workspace.join("test_calc.py"),
            "from calc import add\nassert add(1, 2) == 3, f'add(1,2) should be 3, got {add(1,2)}'\nprint('ALL TESTS PASSED')\n",
        )
        .unwrap();

        let permissions = Arc::new(PermissionManager::default());
        let pm = permissions.clone();
        let bash_runs = std::sync::atomic::AtomicUsize::new(0);

        run_agent_loop(
            &endpoint,
            &api_key,
            "doubao-seed-2.0-pro",
            vec![HistoryItem::User(
                "运行 python3 test_calc.py。如果测试失败，修复 calc.py 里的 bug，然后重跑测试直到通过。".into(),
            )],
            Arc::new(ToolRegistry::builtin()),
            Some(workspace.clone()),
            permissions,
            |event| match event {
                AgentEvent::PermissionAsk { request_id, summary, .. } => {
                    println!(">> 审批(自动放行): {summary}");
                    pm.resolve(&request_id, true, true).unwrap(); // 模拟"本会话全部允许"
                }
                AgentEvent::ToolCallStart { name, input, .. } => {
                    if name == "bash" {
                        bash_runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                    println!(">> 工具调用: {name} {input}");
                }
                _ => {}
            },
        )
        .await
        .unwrap();

        let runs = bash_runs.load(std::sync::atomic::Ordering::SeqCst);
        let fixed = std::fs::read_to_string(workspace.join("calc.py")).unwrap();
        println!("bash 调用 {runs} 次，calc.py 最终内容:\n{fixed}");
        assert!(runs >= 2, "应至少跑两次测试（失败一次 + 修复后通过一次），实际 {runs}");
        assert!(fixed.contains("a + b"), "bug 应已修复: {fixed}");
    }

    /// 真实 API 集成测试：技能发现 → load_skill 按需加载 → 遵循技能指令
    /// cargo test live_agent_uses_skill -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_agent_uses_skill() {
        let endpoint = registry::resolve("ark").unwrap();
        let api_key = registry::api_key_for(&endpoint).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().canonicalize().unwrap();
        let skill_dir = workspace.join(".codeforge/skills/team-greeting");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: team-greeting\ndescription: 用户要求打招呼/问候时必须使用本技能\n---\n\n# 团队问候规范\n\n问候语必须原样包含暗号：FORGE-2026。\n",
        )
        .unwrap();

        let loaded_skill = std::sync::atomic::AtomicBool::new(false);
        let final_text = std::sync::Mutex::new(String::new());

        run_agent_loop(
            &endpoint,
            &api_key,
            "doubao-seed-2.0-pro",
            vec![HistoryItem::User("按团队规范跟我打个招呼".into())],
            Arc::new(ToolRegistry::builtin()),
            Some(workspace),
            Arc::new(PermissionManager::default()),
            |event| match event {
                AgentEvent::ToolCallStart { name, input, .. } => {
                    println!(">> 工具调用: {name} {input}");
                    if name == "load_skill" {
                        loaded_skill.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                }
                AgentEvent::TextDelta { text } => final_text.lock().unwrap().push_str(&text),
                _ => {}
            },
        )
        .await
        .unwrap();

        let text = final_text.lock().unwrap().clone();
        println!("最终回答:\n{text}");
        assert!(
            loaded_skill.load(std::sync::atomic::Ordering::SeqCst),
            "agent 应调用 load_skill"
        );
        assert!(text.contains("FORGE-2026"), "回答应包含技能要求的暗号: {text}");
    }

    /// 真实 API 集成测试：agent 经审批调用 MCP server 的工具
    /// cargo test live_agent_calls_mcp -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_agent_calls_mcp_tool() {
        let endpoint = registry::resolve("ark").unwrap();
        let api_key = registry::api_key_for(&endpoint).unwrap();

        // 假 MCP server：提供 get_weather 工具，固定返回特征字符串
        let script = r#"
import sys, json
for line in sys.stdin:
    msg = json.loads(line)
    mid = msg.get("id"); method = msg.get("method", "")
    if method == "initialize":
        out = {"jsonrpc":"2.0","id":mid,"result":{"protocolVersion":"2024-11-05","serverInfo":{"name":"weather"}}}
    elif method == "tools/list":
        out = {"jsonrpc":"2.0","id":mid,"result":{"tools":[{"name":"get_weather","description":"查询某城市当前天气","inputSchema":{"type":"object","properties":{"city":{"type":"string"}},"required":["city"]}}]}}
    elif method == "tools/call":
        city = msg["params"]["arguments"]["city"]
        out = {"jsonrpc":"2.0","id":mid,"result":{"content":[{"type":"text","text":city+" 当前天气：晴，26 度，风速 MCP-7"}]}}
    elif mid is None:
        continue
    else:
        out = {"jsonrpc":"2.0","id":mid,"error":{"code":-32601,"message":"unknown"}}
    sys.stdout.write(json.dumps(out)+"\n"); sys.stdout.flush()
"#;
        let dir = tempfile::tempdir().unwrap();
        let script_path = dir.path().join("weather_mcp.py");
        std::fs::write(&script_path, script).unwrap();

        let connection = Arc::new(
            crate::mcp::McpConnection::connect(
                "weather",
                &crate::mcp::McpServerConfig {
                    command: "python3".into(),
                    args: vec![script_path.to_string_lossy().to_string()],
                    env: Default::default(),
                },
            )
            .unwrap(),
        );

        // 内置 + MCP 工具组装注册表（与 send_message 同款逻辑）
        let mut tools = ToolRegistry::builtin().all();
        tools.extend(crate::tools::mcp_adapter::McpToolAdapter::wrap_all(&connection));
        let registry_combined = Arc::new(ToolRegistry::from_tools(tools));

        let permissions = Arc::new(PermissionManager::default());
        let pm = permissions.clone();
        let mcp_called = std::sync::atomic::AtomicBool::new(false);
        let final_text = std::sync::Mutex::new(String::new());

        run_agent_loop(
            &endpoint,
            &api_key,
            "doubao-seed-2.0-pro",
            vec![HistoryItem::User("用工具查一下杭州现在的天气".into())],
            registry_combined,
            Some(dir.path().canonicalize().unwrap()),
            permissions,
            |event| match event {
                AgentEvent::PermissionAsk { request_id, summary, .. } => {
                    println!(">> 审批(自动放行): {summary}");
                    pm.resolve(&request_id, true, false).unwrap();
                }
                AgentEvent::ToolCallStart { name, input, .. } => {
                    println!(">> 工具调用: {name} {input}");
                    if name.starts_with("mcp__weather__") {
                        mcp_called.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                }
                AgentEvent::TextDelta { text } => final_text.lock().unwrap().push_str(&text),
                _ => {}
            },
        )
        .await
        .unwrap();

        let text = final_text.lock().unwrap().clone();
        println!("最终回答:\n{text}");
        assert!(mcp_called.load(std::sync::atomic::Ordering::SeqCst), "应调用 MCP 工具");
        assert!(text.contains("MCP-7"), "回答应包含 MCP 工具返回的特征值: {text}");
    }
}
