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
use crate::llm::types::{AssistantTurn, HistoryItem, LlmDelta, ToolCall, ToolSpec};
use crate::llm::{anthropic, openai};
use crate::security::PermissionManager;
use crate::tools::registry::ToolRegistry;

const MAX_ITERATIONS: usize = 30;
/// 工具结果回传前端展示时的截断长度（回填给模型的是全量）
const EVENT_OUTPUT_PREVIEW_CHARS: usize = 2000;
const SUBAGENT_TOOL: &str = "spawn_subagents";
const MAX_SUBAGENTS: usize = 4;

/// 交互模式：决定审批策略和可用工具集
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AgentMode {
    /// 默认：写文件/跑命令都要审批
    Ask,
    /// 自动：非危险操作直接放行，仅危险动作(rm -rf 等)弹审批
    Auto,
    /// 计划：禁用一切写/执行工具，只读+调研，产出方案
    Plan,
}

impl AgentMode {
    pub fn parse(s: &str) -> Self {
        match s {
            "auto" => AgentMode::Auto,
            "plan" => AgentMode::Plan,
            _ => AgentMode::Ask,
        }
    }
}

/// 一次 agent 运行的环境（主/子 agent 共享，子 agent 直接复用引用）
pub struct AgentCtx {
    pub endpoint: Endpoint,
    pub api_key: String,
    pub model: String,
    pub registry: Arc<ToolRegistry>,
    pub workspace: Option<PathBuf>,
    pub permissions: Arc<PermissionManager>,
    /// 交互模式
    pub mode: AgentMode,
    /// 停止按钮：置 true 后流式读取、loop 迭代、子 agent 都会尽快收尾
    pub cancel: Arc<std::sync::atomic::AtomicBool>,
    /// provider id（trace 的 gen_ai.system 属性）
    pub provider: String,
    /// 本地 OTel trace 写入器（无 session 时为 None）
    pub trace: Option<Arc<crate::trace::TraceWriter>>,
    /// agent 的任务清单（todo_write 维护，每轮作为 system-reminder 注入）
    pub todos: crate::tools::todo::TodoList,
    /// 检查点存储位置 + 会话 id（无会话则不快照）
    pub app_data: Option<PathBuf>,
    pub session_id: Option<i64>,
    /// 生成中用户追加的消息队列（loop 每轮注入）
    pub pending: Arc<std::sync::Mutex<Vec<String>>>,
    /// 当前 agent 角色（会话驱动选中的，或子 agent 被指派的）；None=通用 agent
    pub role: Option<Arc<crate::agents::AgentRole>>,
}

impl AgentCtx {
    /// 复制一份上下文、换一个角色，用于派发带角色的子 agent。
    /// 共享的状态（registry/permissions/cancel/trace/pending 等）按 Arc 浅拷贝。
    fn child_with_role(&self, role: Option<Arc<crate::agents::AgentRole>>) -> AgentCtx {
        AgentCtx {
            endpoint: self.endpoint,
            api_key: self.api_key.clone(),
            model: self.model.clone(),
            registry: self.registry.clone(),
            workspace: self.workspace.clone(),
            permissions: self.permissions.clone(),
            mode: self.mode,
            cancel: self.cancel.clone(),
            provider: self.provider.clone(),
            trace: self.trace.clone(),
            todos: self.todos.clone(),
            app_data: self.app_data.clone(),
            session_id: self.session_id,
            pending: self.pending.clone(),
            role,
        }
    }
}

fn drain_pending(ctx: &AgentCtx) -> Vec<String> {
    std::mem::take(&mut *ctx.pending.lock().unwrap())
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

/// 写文件类工具执行前拍快照，返回检查点 id。条件不满足（非写工具/无会话/无工作区）返回 None。
fn maybe_capture_checkpoint(
    ctx: &AgentCtx,
    event_id: &str,
    tool_name: &str,
    input: &Value,
) -> Option<String> {
    let (app_data, session_id, workspace) = (
        ctx.app_data.as_ref()?,
        ctx.session_id?,
        ctx.workspace.as_ref()?,
    );
    let tool = ctx.registry.get(tool_name)?;
    let paths = tool.affected_paths(input);
    if paths.is_empty() {
        return None;
    }
    let checkpoint_id = ctx
        .trace
        .as_ref()
        .map(|t| t.reserve_span_id())
        .unwrap_or_else(|| format!("cp{}", crate::trace::now_unix_nanos()));
    let label = format!("{tool_name} {}", paths.join(", "));
    crate::checkpoint::capture(
        app_data, session_id, &checkpoint_id, event_id, &label, workspace, &paths,
    )
    .ok()
    .map(|_| checkpoint_id)
}

fn cap_chars(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_string()
    } else {
        format!("{}…[+{}字符]", text.chars().take(limit).collect::<String>(), text.chars().count() - limit)
    }
}

fn subagent_spec(workspace: Option<&std::path::Path>) -> ToolSpec {
    // 把可用角色列进描述，主 agent 才知道能给子任务指派角色
    let roles = crate::agents::discover(workspace);
    let role_list = roles
        .iter()
        .map(|r| format!("{}（{}）", r.name, r.description))
        .collect::<Vec<_>>()
        .join("；");
    let desc = format!(
        "把 1~4 个互相独立的子任务并行派给子 agent。每个子 agent 有独立上下文，最终只把书面汇报返回给你。\
适合并行探索/批量调查/分工（如让 review 角色审、research 角色查）；不适合有先后依赖的步骤。\
可给每个子任务指定 role 让它以特定角色+工具集执行。可用角色：{role_list}。"
    );
    ToolSpec {
        name: SUBAGENT_TOOL.into(),
        description: desc,
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
                            "prompt": {"type": "string", "description": "给子 agent 的完整任务说明（它没有你的上下文，写清楚背景和期望产出）"},
                            "role": {"type": "string", "description": "可选：让子 agent 以某个角色执行（research/product/dev/review 等），不填则用通用 agent"}
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
        let plan_only = ctx.mode == AgentMode::Plan;
        // 本轮用户问题（取初始历史里最后一条 User），喂给记忆做相关性检索
        let query = history.iter().rev().find_map(|h| match h {
            HistoryItem::User(t) => Some(t.clone()),
            _ => None,
        });
        let role_prompt = ctx.role.as_ref().map(|r| r.system_prompt.as_str());
        let base_system = prompt::build_system_prompt(
            ctx.workspace.as_deref(),
            !is_main,
            ctx.mode,
            query.as_deref(),
            role_prompt,
        );
        let mut tools = ctx.registry.specs(ctx.workspace.is_some(), plan_only);
        // 角色工具白名单：只保留该角色允许的工具（空白名单=不限）
        if let Some(role) = &ctx.role {
            tools.retain(|t| role.allows(&t.name));
        }
        // plan 模式不派子 agent；角色不允许 spawn_subagents 时也不给
        let role_allows_subagents = ctx.role.as_ref().map(|r| r.allows(SUBAGENT_TOOL)).unwrap_or(true);
        if is_main && !plan_only && role_allows_subagents {
            tools.push(subagent_spec(ctx.workspace.as_deref()));
        }

        let mut final_text = String::new();
        // 本轮已 read_file 过的文件（read-before-edit 闸门 + 重复读去重）
        let read_files: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));

        for iteration in 0..MAX_ITERATIONS {
            // 任务清单作为 system-reminder 拼在 system 末尾（随清单变化，故每轮重建）
            let system = match crate::tools::todo::reminder(&ctx.todos) {
                Some(r) => format!("{base_system}\n\n{r}"),
                None => base_system.clone(),
            };
            // 用户在生成过程中追加的消息：注入本轮（仅主 agent）
            if is_main {
                for q in drain_pending(ctx) {
                    history.push(HistoryItem::User(q));
                }
            }
            // 轮内压缩：先微压缩（重复读去重，无损），再按预算清理旧结果
            if iteration > 0 {
                let deduped = dedup_reads(&mut history);
                let pruned = prune_tool_results(&mut history);
                if (deduped + pruned) > 0 && is_main {
                    let mut parts = Vec::new();
                    if deduped > 0 {
                        parts.push(format!("去重 {deduped} 次重复读取"));
                    }
                    if pruned > 0 {
                        parts.push(format!("清理 {pruned} 个超预算的较早结果"));
                    }
                    on_event(AgentEvent::ContextCompacted { note: parts.join("，") });
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
                    Err(e) if e.is_cancelled() => {}
                    Err(e) => {
                        tracer.span(
                            &format!("chat {}", ctx.model),
                            run_span,
                            llm_start,
                            json!({"gen_ai.system": ctx.provider, "gen_ai.request.model": ctx.model}),
                            Some(&e.user_message()),
                        );
                    }
                }
            }
            let turn = match call_result {
                Err(e) if e.is_cancelled() => {
                    if is_main {
                        on_event(AgentEvent::TurnEnd { stop_reason: Some("cancelled".into()), input_tokens: None, output_tokens: None });
                    }
                    return Ok(final_text);
                }
                Err(e) => return Err(e.user_message()),
                Ok(turn) => turn,
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

            // 每次 LLM 调用都上报用量：前端累加 session 总花费、刷新本轮花费和上下文占用条。
            // 子 agent 的调用不直接计入主会话面板（其汇报已折算进父 agent 的后续调用）。
            if is_main {
                if let (Some(ci), Some(co)) = (input_tokens, output_tokens) {
                    on_event(AgentEvent::Usage {
                        call_input: ci,
                        call_output: co,
                        context_tokens: ci,
                        cache_read: turn.cache_read_tokens.unwrap_or(0),
                    });
                }
            }

            history.push(HistoryItem::Assistant {
                text: turn.text,
                tool_calls: tool_calls.clone(),
            });

            if tool_calls.is_empty() {
                // agent 本想结束，但若用户已追加消息，则继续回应（追加对话）
                if is_main {
                    let more = drain_pending(ctx);
                    if !more.is_empty() {
                        for q in more {
                            history.push(HistoryItem::User(q));
                        }
                        continue;
                    }
                }
                on_event(AgentEvent::TurnEnd { stop_reason, input_tokens, output_tokens });
                return Ok(final_text);
            }

            // 工具执行：连续的只读(parallel_safe)工具并发跑，其余串行。保持 history 顺序。
            let is_par = |c: &ToolCall| {
                c.name != SUBAGENT_TOOL
                    && ctx.registry.get(&c.name).map(|t| t.parallel_safe()).unwrap_or(false)
            };
            let mut idx = 0;
            while idx < tool_calls.len() {
                if is_cancelled(ctx) {
                    if is_main {
                        on_event(AgentEvent::TurnEnd { stop_reason: Some("cancelled".into()), input_tokens: None, output_tokens: None });
                    }
                    return Ok(final_text);
                }
                if is_par(&tool_calls[idx]) {
                    // 收集一段连续的只读工具，并发执行
                    let start = idx;
                    while idx < tool_calls.len() && is_par(&tool_calls[idx]) {
                        idx += 1;
                    }
                    let group = &tool_calls[start..idx];
                    if group.len() == 1 {
                        history.push(execute_call(ctx, id_prefix, &group[0], on_event, run_span, is_main, &read_files).await);
                    } else {
                        let results = futures_util::future::join_all(
                            group.iter().map(|c| execute_call(ctx, id_prefix, c, on_event, run_span, is_main, &read_files)),
                        )
                        .await;
                        history.extend(results);
                    }
                } else {
                    history.push(execute_call(ctx, id_prefix, &tool_calls[idx], on_event, run_span, is_main, &read_files).await);
                    idx += 1;
                }
            }
        }

        Err(format!("达到最大迭代次数（{MAX_ITERATIONS}），任务可能过于复杂，请拆小后重试"))
    }
}

/// 执行单个工具调用：发 start 事件 → 快照 → 执行(子agent/普通工具) → trace → 发 end 事件，
/// 返回回填给历史的 ToolResult。被串行与并行两条路径共用。
async fn execute_call(
    ctx: &AgentCtx,
    id_prefix: &str,
    call: &ToolCall,
    on_event: &EventSink<'_>,
    run_span: Option<&str>,
    is_main: bool,
    read_files: &std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
) -> HistoryItem {
    let event_id = format!("{id_prefix}{}", call.id);
    let input: Value =
        serde_json::from_str(&call.arguments).unwrap_or(Value::Object(Default::default()));
    on_event(AgentEvent::ToolCallStart {
        id: event_id.clone(),
        name: call.name.clone(),
        input: input.clone(),
    });

    // 角色工具白名单的执行期兜底：模型若调了本角色不该用的工具，直接拒绝
    if let Some(role) = &ctx.role {
        if !role.allows(&call.name) {
            let rejection = format!(
                "工具 {} 被拒绝：当前是「{}」角色，不允许使用该工具。请在职责范围内完成任务。",
                call.name, role.name
            );
            on_event(AgentEvent::ToolCallEnd {
                id: event_id,
                output: preview(&rejection),
                is_error: true,
                duration_ms: 0,
                checkpoint_id: None,
            });
            return HistoryItem::ToolResult {
                call_id: call.id.clone(),
                name: call.name.clone(),
                content: rejection,
                is_error: true,
            };
        }
    }

    // read-before-edit 闸门：没读过就不准凭记忆改文件，避免幻觉式 edit 把文件改坏
    if let Some(rejection) = {
        let set = read_files.lock().unwrap();
        check_read_before_edit(call, &set)
    } {
        on_event(AgentEvent::ToolCallEnd {
            id: event_id,
            output: preview(&rejection),
            is_error: true,
            duration_ms: 0,
            checkpoint_id: None,
        });
        return HistoryItem::ToolResult {
            call_id: call.id.clone(),
            name: call.name.clone(),
            content: rejection,
            is_error: true,
        };
    }

    let checkpoint_id = maybe_capture_checkpoint(ctx, &event_id, &call.name, &input);
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

    // 成功读过的文件登记进集合，作为后续 edit 的前置许可
    if !is_error && call.name == "read_file" {
        if let Some(path) = tool_path(call) {
            read_files.lock().unwrap().insert(path);
        }
    }
    let checkpoint_id = if is_error { None } else { checkpoint_id };

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
            is_error.then_some(content.as_str()).map(|_| "tool error"),
        );
    }

    on_event(AgentEvent::ToolCallEnd {
        id: event_id,
        output: preview(&content),
        is_error,
        duration_ms: started.elapsed().as_millis() as u64,
        checkpoint_id,
    });
    HistoryItem::ToolResult {
        call_id: call.id.clone(),
        name: call.name.clone(),
        content,
        is_error,
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
    // 每个子任务可带 role 指定角色（research/product/dev/review 或自定义）
    let parsed: Vec<(String, String, Option<String>)> = tasks
        .iter()
        .map(|t| {
            Ok((
                t["title"].as_str().ok_or("子任务缺少 title")?.to_string(),
                t["prompt"].as_str().ok_or("子任务缺少 prompt")?.to_string(),
                t["role"].as_str().filter(|r| !r.is_empty()).map(str::to_string),
            ))
        })
        .collect::<Result<_, String>>()?;

    // 为每个子任务按其 role 准备一份子上下文（角色无效则退回通用 agent）
    let child_ctxs: Vec<AgentCtx> = parsed
        .iter()
        .map(|(_, _, role_name)| {
            let role = role_name
                .as_deref()
                .and_then(|n| crate::agents::resolve(ctx.workspace.as_deref(), n))
                .map(std::sync::Arc::new);
            ctx.child_with_role(role)
        })
        .collect();

    // 子 agent 的文本增量不直接进会话流（只有最终汇报作为工具结果返回），
    // 但工具调用/审批事件照常转发，用户能看到子 agent 在干什么
    let quiet = |event: AgentEvent| match event {
        AgentEvent::TextDelta { .. }
        | AgentEvent::ReasoningDelta { .. }
        | AgentEvent::TurnEnd { .. } => {}
        other => on_event(other),
    };

    let futures = child_ctxs.iter().zip(parsed.iter()).enumerate().map(
        |(index, (cctx, (_, prompt_text, _)))| {
            loop_impl(
                cctx,
                vec![HistoryItem::User(prompt_text.clone())],
                &quiet,
                format!("{parent_id}-s{index}-"),
                false,
                trace_parent.map(str::to_string),
            )
        },
    );
    let results = futures_util::future::join_all(futures).await;

    let mut report = String::new();
    for ((title, _, _), result) in parsed.iter().zip(results) {
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
) -> Result<AssistantTurn, crate::llm::types::LlmError> {
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
        // auto 模式：非危险操作直接放行，仅危险动作弹审批；ask 模式：都弹（会话级"全部允许"除外）
        let needs_approval = match ctx.mode {
            AgentMode::Auto => plan.danger.is_some(),
            _ => true,
        };
        if needs_approval && !ctx.permissions.is_allow_all() {
            // 先注册再发事件，避免决议先于等待到达的竞态
            let rx = ctx.permissions.register(request_id);
            on_event(AgentEvent::PermissionAsk {
                request_id: request_id.to_string(),
                tool_name: name.to_string(),
                summary: plan.summary,
                diff: plan.diff,
                danger: plan.danger,
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

/// 从工具调用里抽出它读/改的文件相对路径（read_file/edit_file/write_file 都有 path 参数）
fn tool_path(call: &ToolCall) -> Option<String> {
    let input: Value = serde_json::from_str(&call.arguments).ok()?;
    input["path"].as_str().map(|s| s.to_string())
}

/// read-before-edit 闸门：edit_file 前必须在本轮 read 过该文件。
/// 给 LLM 一个明确指引而非靠 old_string 匹配失败的隐晦报错。
fn check_read_before_edit(call: &ToolCall, read_files: &std::collections::HashSet<String>) -> Option<String> {
    if call.name != "edit_file" {
        return None;
    }
    let path = tool_path(call)?;
    if read_files.contains(&path) {
        None
    } else {
        Some(format!(
            "edit_file 被拒绝：你还没在本次会话用 read_file 读过 {path}，不能凭记忆修改。请先 read_file {path} 看清当前内容，再发起精确的 edit。"
        ))
    }
}

// 轮内压缩参数
const SINGLE_RESULT_MAX: usize = 16_000; // 单结果超此值 → 中段截断
const SINGLE_RESULT_HEAD: usize = 6_000;
const SINGLE_RESULT_TAIL: usize = 6_000;
const TOOL_RESULT_BUDGET_CHARS: usize = 60_000; // 全部结果总预算
const KEEP_HEAD_RESULTS: usize = 2; // 始终保留最早的几个（项目定位上下文）
const PRUNED_MARK: &str = "[已清理]";

/// 单个超大结果：保留前后两段，砍掉中间
fn middle_truncate(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= SINGLE_RESULT_MAX {
        return s.to_string();
    }
    let head: String = chars[..SINGLE_RESULT_HEAD].iter().collect();
    let tail: String = chars[chars.len() - SINGLE_RESULT_TAIL..].iter().collect();
    let dropped = chars.len() - SINGLE_RESULT_HEAD - SINGLE_RESULT_TAIL;
    format!("{head}\n…[中间 {dropped} 字符已省略，需要完整内容请缩小范围重新调用]…\n{tail}")
}

/// 轮内压缩，两阶段：
/// ① 单结果中段截断（保留头尾）
/// ② 总量超预算 → 保留最早 KEEP_HEAD_RESULTS 个 + 最近的若干，中间整轮替换为占位
/// 返回本次发生压缩的条数。
fn prune_tool_results(history: &mut [HistoryItem]) -> usize {
    let mut pruned = 0usize;

    // 阶段 ①：单结果中段截断
    for item in history.iter_mut() {
        if let HistoryItem::ToolResult { content, .. } = item {
            if !content.starts_with(PRUNED_MARK) && content.chars().count() > SINGLE_RESULT_MAX {
                *content = middle_truncate(content);
                pruned += 1;
            }
        }
    }

    // 阶段 ②：总预算——保护最早几个，从最新往回保留，中间砍掉
    let result_idxs: Vec<usize> = history
        .iter()
        .enumerate()
        .filter(|(_, i)| {
            matches!(i, HistoryItem::ToolResult { content, .. } if !content.starts_with(PRUNED_MARK))
        })
        .map(|(i, _)| i)
        .collect();
    let protect_head: std::collections::HashSet<usize> =
        result_idxs.iter().take(KEEP_HEAD_RESULTS).copied().collect();

    let mut used = 0usize;
    for &i in result_idxs.iter().rev() {
        if protect_head.contains(&i) {
            continue; // 头部始终保留
        }
        if let HistoryItem::ToolResult { content, name, .. } = &mut history[i] {
            let len = content.chars().count();
            if used + len > TOOL_RESULT_BUDGET_CHARS {
                *content =
                    format!("{PRUNED_MARK} 中间步骤 {name} 的结果（{len} 字符）已省略；如仍需要请重新调用该工具");
                pruned += 1;
            } else {
                used += len;
            }
        }
    }
    pruned
}

/// 微压缩：同一文件被 read_file 多次时，只有最后一次内容是当前的。
/// 把更早的那几次结果替换为指向最新版本的占位，无损地省下重复正文。
/// 返回被去重的条数。
fn dedup_reads(history: &mut [HistoryItem]) -> usize {
    // call_id → 它读的文件路径（只看 read_file 调用）
    let mut read_call_path: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for item in history.iter() {
        if let HistoryItem::Assistant { tool_calls, .. } = item {
            for c in tool_calls {
                if c.name == "read_file" {
                    if let Some(p) = tool_path(c) {
                        read_call_path.insert(c.id.clone(), p);
                    }
                }
            }
        }
    }

    // 每个路径最后一次成功 read 的结果下标 → 受保护，其余去重
    let mut latest: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (idx, item) in history.iter().enumerate() {
        if let HistoryItem::ToolResult { call_id, content, is_error, .. } = item {
            if *is_error || content.starts_with(PRUNED_MARK) {
                continue;
            }
            if let Some(path) = read_call_path.get(call_id) {
                latest.insert(path.clone(), idx);
            }
        }
    }

    let mut deduped = 0usize;
    for idx in 0..history.len() {
        let path = match &history[idx] {
            HistoryItem::ToolResult { call_id, content, is_error, .. }
                if !*is_error && !content.starts_with(PRUNED_MARK) =>
            {
                read_call_path.get(call_id).cloned()
            }
            _ => None,
        };
        let Some(path) = path else { continue };
        if latest.get(&path) == Some(&idx) {
            continue; // 最新一次，保留
        }
        if let HistoryItem::ToolResult { content, .. } = &mut history[idx] {
            *content = format!(
                "{PRUNED_MARK} {path} 的这次读取已被后续更新的 read_file 取代，正文以最新一次为准"
            );
            deduped += 1;
        }
    }
    deduped
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
            mode: AgentMode::Ask,
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            provider: "ark".into(),
            trace: None,
            todos: Arc::new(std::sync::Mutex::new(Vec::new())),
            app_data: None,
            session_id: None,
            pending: Arc::new(std::sync::Mutex::new(Vec::new())),
            role: None,
        }
    }

    fn tool_result(name: &str, content: String) -> HistoryItem {
        HistoryItem::ToolResult { call_id: name.into(), name: name.into(), content, is_error: false }
    }

    #[test]
    fn middle_truncates_oversized_single_result() {
        let big = "a".repeat(SINGLE_RESULT_MAX + 10_000);
        let mut history = vec![tool_result("grep", big)];
        let pruned = prune_tool_results(&mut history);
        assert_eq!(pruned, 1);
        match &history[0] {
            HistoryItem::ToolResult { content, .. } => {
                assert!(content.contains("中间"));
                assert!(content.chars().count() < SINGLE_RESULT_MAX + 100);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn keeps_head_and_recent_drops_middle() {
        // 每个 14K（< SINGLE_RESULT_MAX，不触发单结果截断），6 个非头部=84K > 60K 预算
        let chunk = || "y".repeat(14_000);
        let mut history = vec![
            HistoryItem::User("q".into()),
            tool_result("head1", chunk()), // 受保护
            tool_result("head2", chunk()), // 受保护
            tool_result("m1", chunk()),    // 最旧的中间→砍
            tool_result("m2", chunk()),    // →砍
            tool_result("m3", chunk()),    // 最近若干→保留
            tool_result("m4", chunk()),
            tool_result("m5", chunk()),
            tool_result("recent", chunk()),
        ];
        prune_tool_results(&mut history);
        let kept: Vec<bool> = history
            .iter()
            .filter_map(|i| match i {
                HistoryItem::ToolResult { content, .. } => Some(!content.starts_with(PRUNED_MARK)),
                _ => None,
            })
            .collect();
        // 头2保留 + 最旧两个中间被砍 + 最近的保留
        assert_eq!(kept[0], true);
        assert_eq!(kept[1], true);
        assert_eq!(kept[2], false, "最旧的中间应被砍");
        assert_eq!(*kept.last().unwrap(), true, "最近的应保留");
        assert!(kept.iter().filter(|k| !**k).count() >= 1, "至少砍掉一个中间");
        // 幂等
        let before = history.len();
        prune_tool_results(&mut history);
        assert_eq!(history.len(), before);
    }

    fn read_call(id: &str, path: &str) -> ToolCall {
        ToolCall { id: id.into(), name: "read_file".into(), arguments: format!("{{\"path\":\"{path}\"}}") }
    }

    fn read_result(call_id: &str, content: &str) -> HistoryItem {
        HistoryItem::ToolResult {
            call_id: call_id.into(),
            name: "read_file".into(),
            content: content.into(),
            is_error: false,
        }
    }

    #[test]
    fn dedup_reads_keeps_only_latest_per_file() {
        let mut history = vec![
            HistoryItem::Assistant { text: String::new(), tool_calls: vec![read_call("c1", "a.rs")] },
            read_result("c1", "first version of a.rs"),
            HistoryItem::Assistant { text: String::new(), tool_calls: vec![read_call("c2", "b.rs")] },
            read_result("c2", "content of b.rs"),
            HistoryItem::Assistant { text: String::new(), tool_calls: vec![read_call("c3", "a.rs")] },
            read_result("c3", "second version of a.rs"),
        ];
        let n = dedup_reads(&mut history);
        assert_eq!(n, 1, "a.rs 读了两次，第一次应被去重");
        match &history[1] {
            HistoryItem::ToolResult { content, .. } => {
                assert!(content.starts_with(PRUNED_MARK), "旧读取应被替换为占位");
                assert!(content.contains("a.rs"));
            }
            _ => panic!(),
        }
        // b.rs 唯一一次读取，保留
        match &history[3] {
            HistoryItem::ToolResult { content, .. } => assert_eq!(content, "content of b.rs"),
            _ => panic!(),
        }
        // a.rs 最新一次读取，保留
        match &history[5] {
            HistoryItem::ToolResult { content, .. } => assert_eq!(content, "second version of a.rs"),
            _ => panic!(),
        }
        // 幂等
        assert_eq!(dedup_reads(&mut history), 0);
    }

    #[test]
    fn read_before_edit_gate_blocks_unread_file() {
        let edit = ToolCall {
            id: "e1".into(),
            name: "edit_file".into(),
            arguments: "{\"path\":\"x.rs\",\"old_string\":\"a\",\"new_string\":\"b\"}".into(),
        };
        let mut set = std::collections::HashSet::new();
        assert!(check_read_before_edit(&edit, &set).is_some(), "未读过应被拒绝");
        set.insert("x.rs".to_string());
        assert!(check_read_before_edit(&edit, &set).is_none(), "读过后应放行");
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
        let read_skill = std::sync::atomic::AtomicBool::new(false);
        let final_text = std::sync::Mutex::new(String::new());

        run_agent_loop(
            &ctx,
            vec![HistoryItem::User("按团队规范跟我打个招呼".into())],
            &|event| match event {
                AgentEvent::ToolCallStart { name, input, .. } => {
                    println!(">> 工具调用: {name} {input}");
                    // 改用 read_file 读 SKILL.md（不再有 load_skill 工具）
                    if name == "read_file" && input.to_string().contains("SKILL.md") {
                        read_skill.store(true, std::sync::atomic::Ordering::SeqCst);
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
        assert!(read_skill.load(std::sync::atomic::Ordering::SeqCst), "应 read_file 读 SKILL.md");
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
