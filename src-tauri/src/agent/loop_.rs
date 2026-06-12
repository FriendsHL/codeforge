//! agent 核心循环：调 LLM → 解析工具调用 → 执行 → 结果回填 → 再调 LLM，直到无工具调用。
//! 支持 spawn_subagents：把独立子任务并行派给子 agent（独立上下文，深度限 1 层）。

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::{json, Value};

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
const SUBAGENT_TOOL: &str = "spawn_subagents";
const MAX_SUBAGENTS: usize = 4;

/// 一次 agent 运行的环境（主/子 agent 共享，子 agent 直接复用引用）
pub struct AgentCtx {
    pub endpoint: Endpoint,
    pub api_key: String,
    pub model: String,
    pub registry: Arc<ToolRegistry>,
    pub workspace: Option<PathBuf>,
    pub permissions: Arc<PermissionManager>,
    /// 停止按钮：置 true 后流式读取、loop 迭代、子 agent 都会尽快收尾
    pub cancel: Arc<std::sync::atomic::AtomicBool>,
    /// provider id（trace 的 gen_ai.system 属性）
    pub provider: String,
    /// 本地 OTel trace 写入器（无 session 时为 None）
    pub trace: Option<Arc<crate::trace::TraceWriter>>,
}

fn is_cancelled(ctx: &AgentCtx) -> bool {
    ctx.cancel.load(std::sync::atomic::Ordering::Relaxed)
}

/// 事件回调 trait object（带生命周期参数：调用方的闭包可以借用本地变量）
type EventSink<'e> = dyn Fn(AgentEvent) + Sync + 'e;

pub async fn run_agent_loop(
    ctx: &AgentCtx,
    history: Vec<HistoryItem>,
    on_event: &EventSink<'_>,
) -> Result<(), String> {
    loop_impl(ctx, history, on_event, String::new(), true, None)
        .await
        .map(|_| ())
}

fn cap_chars(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_string()
    } else {
        format!("{}…[+{}字符]", text.chars().take(limit).collect::<String>(), text.chars().count() - limit)
    }
}

fn subagent_spec() -> ToolSpec {
    ToolSpec {
        name: SUBAGENT_TOOL.into(),
        description: "把 1~4 个互相独立的子任务并行派给子 agent。每个子 agent 有独立上下文、可用全部工具，最终只把书面汇报返回给你。适合并行探索/批量调查/隔离大输出；不适合有先后依赖的步骤。".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "tasks": {
                    "type": "array",
                    "description": "子任务列表（1~4 个）",
                    "items": {
                        "type": "object",
                        "properties": {
                            "title": {"type": "string", "description": "子任务短标题"},
                            "prompt": {"type": "string", "description": "给子 agent 的完整任务说明（它没有你的上下文，写清楚背景和期望产出）"}
                        },
                        "required": ["title", "prompt"]
                    }
                }
            },
            "required": ["tasks"]
        }),
    }
}

/// 返回本轮 agent 的最终文本（子 agent 用它作汇报）。
/// 递归（子 agent 复用同一循环）需要 Box::pin。
fn loop_impl<'a>(
    ctx: &'a AgentCtx,
    history: Vec<HistoryItem>,
    on_event: &'a EventSink<'a>,
    id_prefix: String,
    is_main: bool,
    trace_parent: Option<String>,
) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>> {
    Box::pin(async move {
        // run 级 span：先预留 id 给子 span 引用，结束时落盘
        let run_start = crate::trace::now_unix_nanos();
        let run_span = ctx.trace.as_ref().map(|t| t.reserve_span_id());

        let result = loop_body(ctx, history, on_event, &id_prefix, is_main, run_span.as_deref()).await;

        if let (Some(tracer), Some(span_id)) = (&ctx.trace, &run_span) {
            tracer.emit(
                span_id,
                if is_main { "agent.run" } else { "agent.subagent.run" },
                trace_parent.as_deref(),
                run_start,
                crate::trace::now_unix_nanos(),
                json!({
                    "gen_ai.system": ctx.provider,
                    "gen_ai.request.model": ctx.model,
                    "codeforge.is_subagent": !is_main,
                }),
                result.as_ref().err().map(|e| e.as_str()),
            );
        }
        result
    })
}

async fn loop_body(
    ctx: &AgentCtx,
    mut history: Vec<HistoryItem>,
    on_event: &EventSink<'_>,
    id_prefix: &str,
    is_main: bool,
    run_span: Option<&str>,
) -> Result<String, String> {
    {
        let system = prompt::build_system_prompt(ctx.workspace.as_deref(), !is_main);
        let mut tools = ctx.registry.specs(ctx.workspace.is_some());
        if is_main {
            tools.push(subagent_spec());
        }

        let mut final_text = String::new();

        for iteration in 0..MAX_ITERATIONS {
            // 轮内压缩：把超出预算的旧工具结果替换为占位（保留"读过什么"的索引）
            if iteration > 0 {
                let pruned = prune_tool_results(&mut history);
                if pruned > 0 && is_main {
                    on_event(AgentEvent::ContextCompacted {
                        note: format!("已清理 {pruned} 个较早的工具结果原文（超出轮内预算）"),
                    });
                }
            }
            if is_cancelled(ctx) {
                if is_main {
                    on_event(AgentEvent::TurnEnd { stop_reason: Some("cancelled".into()), input_tokens: None, output_tokens: None });
                }
                return Ok(final_text);
            }
            let llm_start = crate::trace::now_unix_nanos();
            let call_result = call_llm(ctx, &system, &history, &tools, on_event).await;
            if let Some(tracer) = &ctx.trace {
                match &call_result {
                    Ok(turn) => {
                        tracer.span(
                            &format!("chat {}", ctx.model),
                            run_span,
                            llm_start,
                            json!({
                                "gen_ai.system": ctx.provider,
                                "gen_ai.request.model": ctx.model,
                                "gen_ai.usage.input_tokens": turn.input_tokens,
                                "gen_ai.usage.output_tokens": turn.output_tokens,
                                "gen_ai.response.finish_reasons": [turn.stop_reason.clone()],
                                "codeforge.tool_call_count": turn.tool_calls.len(),
                            }),
                            None,
                        );
                    }
                    Err(e) if e == crate::llm::types::CANCELLED_ERR => {}
                    Err(e) => {
                        tracer.span(
                            &format!("chat {}", ctx.model),
                            run_span,
                            llm_start,
                            json!({"gen_ai.system": ctx.provider, "gen_ai.request.model": ctx.model}),
                            Some(e),
                        );
                    }
                }
            }
            let turn = match call_result {
                Err(e) if e == crate::llm::types::CANCELLED_ERR => {
                    if is_main {
                        on_event(AgentEvent::TurnEnd { stop_reason: Some("cancelled".into()), input_tokens: None, output_tokens: None });
                    }
                    return Ok(final_text);
                }
                other => other?,
            };

            if !turn.text.is_empty() {
                if !final_text.is_empty() {
                    final_text.push_str("\n\n");
                }
                final_text.push_str(&turn.text);
            }
            let stop_reason = turn.stop_reason.clone();
            let input_tokens = turn.input_tokens;
            let output_tokens = turn.output_tokens;
            let tool_calls = turn.tool_calls.clone();

            history.push(HistoryItem::Assistant {
                text: turn.text,
                tool_calls: tool_calls.clone(),
            });

            if tool_calls.is_empty() {
                on_event(AgentEvent::TurnEnd { stop_reason, input_tokens, output_tokens });
                return Ok(final_text);
            }

            for call in tool_calls {
                if is_cancelled(ctx) {
                    if is_main {
                        on_event(AgentEvent::TurnEnd { stop_reason: Some("cancelled".into()), input_tokens: None, output_tokens: None });
                    }
                    return Ok(final_text);
                }
                let event_id = format!("{id_prefix}{}", call.id);
                let input: Value = serde_json::from_str(&call.arguments)
                    .unwrap_or(Value::Object(Default::default()));
                on_event(AgentEvent::ToolCallStart {
                    id: event_id.clone(),
                    name: call.name.clone(),
                    input: input.clone(),
                });

                let started = std::time::Instant::now();
                let tool_start = crate::trace::now_unix_nanos();
                let result = if call.name == SUBAGENT_TOOL {
                    if is_main {
                        run_subagents(ctx, &input, on_event, &event_id, run_span).await
                    } else {
                        Err("子 agent 不允许再派发子 agent".into())
                    }
                } else {
                    execute_tool(ctx, &event_id, &call.name, input.clone(), on_event).await
                };
                let (content, is_error) = match result {
                    Ok(content) => (content, false),
                    Err(message) => (message, true),
                };

                if let Some(tracer) = &ctx.trace {
                    tracer.span(
                        &format!("tool {}", call.name),
                        run_span,
                        tool_start,
                        json!({
                            "gen_ai.tool.name": call.name,
                            "gen_ai.tool.call.id": event_id,
                            "codeforge.tool.input": cap_chars(&call.arguments, 2000),
                            "codeforge.tool.output": cap_chars(&content, 8000),
                            "codeforge.tool.output_chars": content.chars().count(),
                        }),
                        is_error.then_some(content.as_str()).map(|_| "tool error").or(None),
                    );
                }

                on_event(AgentEvent::ToolCallEnd {
                    id: event_id,
                    output: preview(&content),
                    is_error,
                    duration_ms: started.elapsed().as_millis() as u64,
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
}

/// 并行运行子 agent，汇总各自的书面汇报
async fn run_subagents<'a>(
    ctx: &'a AgentCtx,
    input: &Value,
    on_event: &'a EventSink<'a>,
    parent_id: &str,
    trace_parent: Option<&str>,
) -> Result<String, String> {
    let tasks = input["tasks"].as_array().ok_or("缺少 tasks 参数")?;
    if tasks.is_empty() || tasks.len() > MAX_SUBAGENTS {
        return Err(format!("tasks 数量需在 1~{MAX_SUBAGENTS} 之间"));
    }
    let parsed: Vec<(String, String)> = tasks
        .iter()
        .map(|t| {
            Ok((
                t["title"].as_str().ok_or("子任务缺少 title")?.to_string(),
                t["prompt"].as_str().ok_or("子任务缺少 prompt")?.to_string(),
            ))
        })
        .collect::<Result<_, String>>()?;

    // 子 agent 的文本增量不直接进会话流（只有最终汇报作为工具结果返回），
    // 但工具调用/审批事件照常转发，用户能看到子 agent 在干什么
    let quiet = |event: AgentEvent| match event {
        AgentEvent::TextDelta { .. }
        | AgentEvent::ReasoningDelta { .. }
        | AgentEvent::TurnEnd { .. } => {}
        other => on_event(other),
    };

    let futures = parsed.iter().enumerate().map(|(index, (_, prompt_text))| {
        loop_impl(
            ctx,
            vec![HistoryItem::User(prompt_text.clone())],
            &quiet,
            format!("{parent_id}-s{index}-"),
            false,
            trace_parent.map(str::to_string),
        )
    });
    let results = futures_util::future::join_all(futures).await;

    let mut report = String::new();
    for ((title, _), result) in parsed.iter().zip(results) {
        let body = match result {
            Ok(text) if !text.trim().is_empty() => text,
            Ok(_) => "(子 agent 未给出汇报)".into(),
            Err(e) => format!("(子 agent 执行失败: {e})"),
        };
        report.push_str(&format!("## 子任务: {title}\n{body}\n\n"));
    }
    Ok(report.trim_end().to_string())
}

async fn call_llm(
    ctx: &AgentCtx,
    system: &str,
    history: &[HistoryItem],
    tools: &[ToolSpec],
    on_event: &EventSink<'_>,
) -> Result<AssistantTurn, String> {
    let on_delta = |delta: LlmDelta| match delta {
        LlmDelta::Text(text) => on_event(AgentEvent::TextDelta { text }),
        LlmDelta::Reasoning(text) => on_event(AgentEvent::ReasoningDelta { text }),
    };
    match ctx.endpoint {
        Endpoint::Anthropic => {
            anthropic::stream_chat(
                &ctx.api_key,
                &ctx.model,
                Some(system),
                history,
                tools,
                &ctx.cancel,
                on_delta,
            )
            .await
        }
        Endpoint::OpenAiCompatible { chat_url, .. } => {
            openai::stream_chat(
                chat_url,
                &ctx.api_key,
                &ctx.model,
                Some(system),
                history,
                tools,
                &ctx.cancel,
                on_delta,
            )
            .await
        }
    }
}

async fn execute_tool(
    ctx: &AgentCtx,
    request_id: &str,
    name: &str,
    input: Value,
    on_event: &EventSink<'_>,
) -> Result<String, String> {
    let Some(tool) = ctx.registry.get(name) else {
        return Err(format!("未知工具: {name}"));
    };
    // 不依赖工作区的工具（联网/技能/MCP）在纯聊天模式下用临时目录兜底
    let workspace = match &ctx.workspace {
        Some(ws) => ws.clone(),
        None if !tool.needs_workspace() => std::env::temp_dir(),
        None => return Err("该工具需要先打开项目目录".into()),
    };

    // 副作用工具先预演 diff/命令走审批
    let plan = {
        let tool = tool.clone();
        let workspace = workspace.clone();
        let input = input.clone();
        tokio::task::spawn_blocking(move || tool.plan(&workspace, &input))
            .await
            .map_err(|e| format!("工具执行崩溃: {e}"))??
    };
    if let Some(plan) = plan {
        if !ctx.permissions.is_allow_all() {
            // 先注册再发事件，避免决议先于等待到达的竞态
            let rx = ctx.permissions.register(request_id);
            on_event(AgentEvent::PermissionAsk {
                request_id: request_id.to_string(),
                tool_name: name.to_string(),
                summary: plan.summary,
                diff: plan.diff,
            });
            if !ctx.permissions.wait(request_id, rx).await {
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
    let request_id = request_id.to_string();
    while let Some(chunk) = chunk_rx.recv().await {
        on_event(AgentEvent::CommandOutput { id: request_id.clone(), chunk });
    }
    handle.await.map_err(|e| format!("工具执行崩溃: {e}"))?
}

/// 轮内工具结果预算：从最新往回数，超出后旧结果原文替换为占位。
/// 返回本次清理的条数。
const TOOL_RESULT_BUDGET_CHARS: usize = 60_000;
const PRUNED_MARK: &str = "[已清理]";

fn prune_tool_results(history: &mut [HistoryItem]) -> usize {
    let mut used = 0usize;
    let mut pruned = 0usize;
    for item in history.iter_mut().rev() {
        if let HistoryItem::ToolResult { content, name, .. } = item {
            if content.starts_with(PRUNED_MARK) {
                continue;
            }
            let len = content.chars().count();
            if used + len > TOOL_RESULT_BUDGET_CHARS {
                *content = format!(
                    "{PRUNED_MARK} {name} 的输出（{len} 字符）已超出上下文预算被移除；如仍需要请重新调用该工具"
                );
                pruned += 1;
            } else {
                used += len;
            }
        }
    }
    pruned
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

    fn ark_ctx(workspace: Option<PathBuf>) -> AgentCtx {
        let endpoint = registry::resolve("ark").unwrap();
        let api_key = registry::api_key_for(&endpoint).unwrap();
        AgentCtx {
            endpoint,
            api_key,
            model: "doubao-seed-2.0-pro".into(),
            registry: Arc::new(ToolRegistry::builtin()),
            workspace,
            permissions: Arc::new(PermissionManager::default()),
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            provider: "ark".into(),
            trace: None,
        }
    }

    #[test]
    fn prunes_old_tool_results_over_budget() {
        let big = "x".repeat(TOOL_RESULT_BUDGET_CHARS);
        let mut history = vec![
            HistoryItem::User("q".into()),
            HistoryItem::ToolResult {
                call_id: "c1".into(),
                name: "read_file".into(),
                content: "旧结果".repeat(100),
                is_error: false,
            },
            HistoryItem::ToolResult {
                call_id: "c2".into(),
                name: "grep".into(),
                content: big.clone(),
                is_error: false,
            },
        ];
        let pruned = prune_tool_results(&mut history);
        assert_eq!(pruned, 1, "旧的应被清理，新的（占满预算）保留");
        match &history[1] {
            HistoryItem::ToolResult { content, .. } => {
                assert!(content.starts_with(PRUNED_MARK));
                assert!(content.contains("read_file"));
            }
            _ => panic!(),
        }
        match &history[2] {
            HistoryItem::ToolResult { content, .. } => assert_eq!(content, &big),
            _ => panic!(),
        }
        // 幂等：再跑不重复清理
        assert_eq!(prune_tool_results(&mut history), 0);
    }

    /// 真实 API 集成测试：cargo test live_agent_loop -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_agent_loop_reads_own_codebase() {
        let workspace = std::env::current_dir().unwrap().parent().unwrap().to_path_buf();
        let ctx = ark_ctx(Some(workspace));

        let tool_call_count = std::sync::atomic::AtomicUsize::new(0);
        let final_text = std::sync::Mutex::new(String::new());

        run_agent_loop(
            &ctx,
            vec![HistoryItem::User(
                "这个项目 Rust 端实现了哪几个 agent 工具？只列工具名。".into(),
            )],
            &|event| match event {
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
        assert!(tool_call_count.load(std::sync::atomic::Ordering::SeqCst) > 0);
        assert!(text.contains("read_file") && text.contains("grep"));
    }

    /// cargo test live_agent_edits -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_agent_edits_file_after_approval() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().canonicalize().unwrap();
        std::fs::write(workspace.join("notes.txt"), "hello codeforge\n").unwrap();

        let ctx = ark_ctx(Some(workspace.clone()));
        let pm = ctx.permissions.clone();
        let asked = std::sync::atomic::AtomicBool::new(false);

        run_agent_loop(
            &ctx,
            vec![HistoryItem::User(
                "把 notes.txt 里的 hello 改成 goodbye，其他内容不动。".into(),
            )],
            &|event| match event {
                AgentEvent::PermissionAsk { request_id, summary, diff, .. } => {
                    println!(">> 审批请求: {summary}\n{diff}");
                    asked.store(true, std::sync::atomic::Ordering::SeqCst);
                    pm.resolve(&request_id, true, false).unwrap();
                }
                AgentEvent::ToolCallStart { name, input, .. } => {
                    println!(">> 工具调用: {name} {input}");
                }
                _ => {}
            },
        )
        .await
        .unwrap();

        assert!(asked.load(std::sync::atomic::Ordering::SeqCst));
        let content = std::fs::read_to_string(workspace.join("notes.txt")).unwrap();
        assert_eq!(content, "goodbye codeforge\n");
    }

    /// cargo test live_agent_self_corrects -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_agent_self_corrects_with_bash() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().canonicalize().unwrap();
        std::fs::write(workspace.join("calc.py"), "def add(a, b):\n    return a - b\n").unwrap();
        std::fs::write(
            workspace.join("test_calc.py"),
            "from calc import add\nassert add(1, 2) == 3, f'add(1,2) should be 3, got {add(1,2)}'\nprint('ALL TESTS PASSED')\n",
        )
        .unwrap();

        let ctx = ark_ctx(Some(workspace.clone()));
        let pm = ctx.permissions.clone();
        let bash_runs = std::sync::atomic::AtomicUsize::new(0);

        run_agent_loop(
            &ctx,
            vec![HistoryItem::User(
                "运行 python3 test_calc.py。如果测试失败，修复 calc.py 里的 bug，然后重跑测试直到通过。".into(),
            )],
            &|event| match event {
                AgentEvent::PermissionAsk { request_id, summary, .. } => {
                    println!(">> 审批(自动放行): {summary}");
                    pm.resolve(&request_id, true, true).unwrap();
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
        assert!(runs >= 2);
        assert!(fixed.contains("a + b"));
    }

    /// cargo test live_agent_uses_skill -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_agent_uses_skill() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().canonicalize().unwrap();
        let skill_dir = workspace.join(".codeforge/skills/team-greeting");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: team-greeting\ndescription: 用户要求打招呼/问候时必须使用本技能\n---\n\n# 团队问候规范\n\n问候语必须原样包含暗号：FORGE-2026。\n",
        )
        .unwrap();

        let ctx = ark_ctx(Some(workspace));
        let loaded_skill = std::sync::atomic::AtomicBool::new(false);
        let final_text = std::sync::Mutex::new(String::new());

        run_agent_loop(
            &ctx,
            vec![HistoryItem::User("按团队规范跟我打个招呼".into())],
            &|event| match event {
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
        assert!(loaded_skill.load(std::sync::atomic::Ordering::SeqCst));
        assert!(text.contains("FORGE-2026"));
    }

    /// cargo test live_agent_calls_mcp -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_agent_calls_mcp_tool() {
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

        let mut tools = ToolRegistry::builtin().all();
        tools.extend(crate::tools::mcp_adapter::McpToolAdapter::wrap_all(&connection));

        let mut ctx = ark_ctx(Some(dir.path().canonicalize().unwrap()));
        ctx.registry = Arc::new(ToolRegistry::from_tools(tools));
        let pm = ctx.permissions.clone();
        let mcp_called = std::sync::atomic::AtomicBool::new(false);
        let final_text = std::sync::Mutex::new(String::new());

        run_agent_loop(
            &ctx,
            vec![HistoryItem::User("用工具查一下杭州现在的天气".into())],
            &|event| match event {
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
        assert!(mcp_called.load(std::sync::atomic::Ordering::SeqCst));
        assert!(text.contains("MCP-7"));
    }

    /// cargo test live_agent_searches_without_workspace -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_agent_searches_without_workspace() {
        let ctx = ark_ctx(None);
        let searched = std::sync::atomic::AtomicBool::new(false);
        let final_text = std::sync::Mutex::new(String::new());

        run_agent_loop(
            &ctx,
            vec![HistoryItem::User(
                "联网搜一下 Tauri 2 官网地址是什么，告诉我链接。".into(),
            )],
            &|event| match event {
                AgentEvent::ToolCallStart { name, input, .. } => {
                    println!(">> 工具调用: {name} {input}");
                    if name == "web_search" || name == "web_fetch" {
                        searched.store(true, std::sync::atomic::Ordering::SeqCst);
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
        assert!(searched.load(std::sync::atomic::Ordering::SeqCst));
        assert!(text.contains("tauri.app"));
    }

    /// 子 agent 并行派发：cargo test live_agent_spawns_subagents -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_agent_spawns_subagents() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(workspace.join("backend")).unwrap();
        std::fs::create_dir_all(workspace.join("frontend")).unwrap();
        std::fs::write(
            workspace.join("backend/main.rs"),
            "// 后端入口：axum HTTP 服务，监听 8080\nfn main() {}\n",
        )
        .unwrap();
        std::fs::write(
            workspace.join("frontend/app.tsx"),
            "// 前端入口：React 应用，调用 /api/v1\nexport default function App() {}\n",
        )
        .unwrap();

        let ctx = ark_ctx(Some(workspace));
        let pm = ctx.permissions.clone();
        let spawned = std::sync::atomic::AtomicBool::new(false);
        let sub_tool_calls = std::sync::atomic::AtomicUsize::new(0);
        let final_text = std::sync::Mutex::new(String::new());

        run_agent_loop(
            &ctx,
            vec![HistoryItem::User(
                "用 spawn_subagents 并行派两个子 agent：一个调查 backend 目录、一个调查 frontend 目录，各自汇报里面是什么技术栈，最后你汇总。".into(),
            )],
            &|event| match event {
                AgentEvent::PermissionAsk { request_id, .. } => {
                    pm.resolve(&request_id, true, true).unwrap(); // 自动放行，避免测试等待
                }
                AgentEvent::ToolCallStart { id, name, input } => {
                    println!(">> 工具调用[{id}]: {name} {input}");
                    if name == "spawn_subagents" {
                        spawned.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                    if id.contains("-s") {
                        sub_tool_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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
        assert!(spawned.load(std::sync::atomic::Ordering::SeqCst), "应调用 spawn_subagents");
        assert!(
            sub_tool_calls.load(std::sync::atomic::Ordering::SeqCst) >= 2,
            "子 agent 应有自己的工具调用"
        );
        assert!(
            text.contains("axum") && text.contains("React"),
            "汇总应包含两个子 agent 的发现: {text}"
        );
    }
}
