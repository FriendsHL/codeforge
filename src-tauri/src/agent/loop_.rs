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
const SPAWN_TEAM_TOOL: &str = "spawn_team";
const TEAM_STATUS_TOOL: &str = "team_status";
const REPORT_TOOL: &str = "report_to_coordinator";
const INSTRUCT_TOOL: &str = "instruct_agent";
const CANCEL_TOOL: &str = "cancel_agent";
/// 后台任务的墙钟兜底超时（粗粒度，只为兜住卡死的任务）。
/// 故意 > bash 自身 max timeout(600s)，让 shell 命令先自我超时返回，不留孤儿进程。
const TASK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1800);
/// 全局同时运行的后台团队任务上限
const MAX_CONCURRENT_TEAM: usize = 8;
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
    /// 异步团队任务注册表（spawn_team/team_status 共用，跨回合存活）
    pub team: Arc<crate::agent::team::TeamRegistry>,
    /// 后台任务用的 'static 事件发射器（背景 agent 完成时推 TeamUpdate）；
    /// 用通用闭包避免和 tauri 耦合。None=无前端通道（如测试）。
    pub bg_events: Option<Arc<dyn Fn(AgentEvent) + Send + Sync>>,
    /// 若本 agent 是 spawn_team 派出的后台任务，这里是它的身份（id+标题），
    /// 据此用 report_to_coordinator 向主 agent 汇报。None=主 agent 或同步子 agent。
    pub team_task: Option<crate::agent::team::TeamTaskHandle>,
}

impl AgentCtx {
    /// 复制一份上下文、换一个角色，用于派发带角色的子 agent。
    /// 共享的状态（registry/permissions/cancel/trace/pending 等）按 Arc 浅拷贝。
    fn child_with_role(&self, role: Option<Arc<crate::agents::AgentRole>>) -> AgentCtx {
        // 角色可覆盖模型（裸 id，同 provider 内）；否则沿用父模型
        let model = role
            .as_ref()
            .and_then(|r| r.model.clone())
            .unwrap_or_else(|| self.model.clone());
        AgentCtx {
            endpoint: self.endpoint,
            api_key: self.api_key.clone(),
            model,
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
            team: self.team.clone(),
            bg_events: self.bg_events.clone(),
            team_task: None, // 派生子上下文默认非团队任务；spawn_team 会显式设置
        }
    }
}

fn drain_pending(ctx: &AgentCtx) -> Vec<String> {
    std::mem::take(&mut *ctx.pending.lock().unwrap())
}

fn is_cancelled(ctx: &AgentCtx) -> bool {
    // 全局停止；或后台团队任务被单独请求取消（cancel_requested 是 sticky 的，
    // 不会被新回合的 cancel.store(false) 抹掉）。
    ctx.cancel.load(std::sync::atomic::Ordering::Relaxed)
        || ctx
            .team_task
            .as_ref()
            .map(|h| ctx.team.is_cancel_requested(&h.id))
            .unwrap_or(false)
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

fn spawn_team_spec() -> ToolSpec {
    ToolSpec {
        name: SPAWN_TEAM_TOOL.into(),
        description: "把若干任务派给后台 agent 并行执行，立即返回各自的 task_id（不阻塞）。\
与 spawn_subagents 的区别：spawn_subagents 会等全部跑完才返回结果；spawn_team 是「派完就走」，\
你可以先干别的，之后用 team_status 查进度/收结果。适合长耗时、可并行、不必马上要结果的活。\
每个任务可指定 role（research/product/dev/review/test 等）。\
派出的后台 agent 可主动向你汇报关键进展（你会在后续步骤看到「📨…汇报」），看到后可据此调整或收口。".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "tasks": {
                    "type": "array",
                    "description": "要派发的后台任务（1~8 个）",
                    "items": {
                        "type": "object",
                        "properties": {
                            "title": {"type": "string", "description": "任务短标题"},
                            "prompt": {"type": "string", "description": "给后台 agent 的完整任务说明（它没有你的上下文）"},
                            "role": {"type": "string", "description": "可选：以某角色执行"}
                        },
                        "required": ["title", "prompt"]
                    }
                }
            },
            "required": ["tasks"]
        }),
    }
}

fn team_status_spec() -> ToolSpec {
    ToolSpec {
        name: TEAM_STATUS_TOOL.into(),
        description: "查询 spawn_team 派发的后台任务状态与结果。不带参数=查全部；带 ids 只查指定任务。\
已完成的任务会带回它的书面汇报。派发后用它收口。".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "ids": {"type": "array", "items": {"type": "string"}, "description": "只查这些 task_id（省略=全部）"}
            },
            "required": []
        }),
    }
}

fn instruct_spec() -> ToolSpec {
    ToolSpec {
        name: INSTRUCT_TOOL.into(),
        description: "给一个正在运行的后台团队任务（spawn_team 派出的）中途下达指令：纠偏、补充上下文、或让它收尾。\
该子 agent 会在下一步抽到你的指令并据此调整。用 team_status 拿 task_id。只对运行中的任务有效。".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "目标后台任务的 task_id"},
                "content": {"type": "string", "description": "给它的指令内容"}
            },
            "required": ["id", "content"]
        }),
    }
}

fn cancel_spec() -> ToolSpec {
    ToolSpec {
        name: CANCEL_TOOL.into(),
        description: "请求取消一个正在运行的后台团队任务（spawn_team 派出的）。best-effort：任务会在下个检查点收尾标记为「已取消」；正卡在长命令上的任务可能要等命令自身超时才停。用 team_status 拿 task_id。".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "要取消的后台任务 task_id"}
            },
            "required": ["id"]
        }),
    }
}

fn report_spec() -> ToolSpec {
    ToolSpec {
        name: REPORT_TOOL.into(),
        description: "向主 agent（协调者）主动汇报：进度、关键发现、需要的决策，或遇到的障碍。\
不必等到任务结束——跑到关键节点就可以报。主 agent 会在下一步看到你的消息并可能调整方向。".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "content": {"type": "string", "description": "要汇报给主 agent 的内容（写清楚，它没有你的上下文）"}
            },
            "required": ["content"]
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
        let role_allows = |t: &str| ctx.role.as_ref().map(|r| r.allows(t)).unwrap_or(true);
        if is_main && !plan_only && role_allows(SUBAGENT_TOOL) {
            tools.push(subagent_spec(ctx.workspace.as_deref()));
        }
        // 异步团队编排工具（仅主 agent、非 plan、角色允许时）
        if is_main && !plan_only && role_allows(SPAWN_TEAM_TOOL) {
            tools.push(spawn_team_spec());
            tools.push(team_status_spec());
            tools.push(instruct_spec());
            tools.push(cancel_spec());
        }
        // 后台团队任务才有的「向协调者汇报」工具
        if ctx.team_task.is_some() {
            tools.push(report_spec());
        }

        let mut final_text = String::new();
        // 本轮已 read_file 过的文件（read-before-edit 闸门 + 重复读去重）
        let read_files: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));

        // 角色可覆盖最大迭代轮数（maxTurns），否则用默认
        let max_iter = ctx.role.as_ref().and_then(|r| r.max_turns).unwrap_or(MAX_ITERATIONS);
        for iteration in 0..max_iter {
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
                // 协调者模式：抽取后台子 agent 的主动汇报，注入上下文供主 agent 反应
                for msg in ctx.team.drain_inbox() {
                    let line = format!(
                        "📨 来自后台团队任务 {}「{}」的汇报：{}",
                        msg.from_id, msg.from_title, msg.content
                    );
                    on_event(AgentEvent::TeamMessage {
                        from_id: msg.from_id,
                        from_title: msg.from_title,
                        content: msg.content,
                    });
                    history.push(HistoryItem::User(line));
                }
            } else if let Some(handle) = &ctx.team_task {
                // 后台团队 agent：抽取主 agent 中途下达的指令，纳入当前工作
                for instruction in ctx.team.drain_agent_mailbox(&handle.id) {
                    history.push(HistoryItem::User(format!("📩 协调者指令：{instruction}")));
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

        Err(format!("达到最大迭代次数（{max_iter}），任务可能过于复杂，请拆小后重试"))
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
    } else if call.name == SPAWN_TEAM_TOOL {
        if is_main {
            spawn_team(ctx, &input, on_event)
        } else {
            Err("子 agent 不允许再派发团队任务".into())
        }
    } else if call.name == TEAM_STATUS_TOOL {
        Ok(team_status(ctx, &input, on_event))
    } else if call.name == CANCEL_TOOL {
        if is_main {
            let id = input["id"].as_str().unwrap_or("").trim();
            if id.is_empty() {
                Err("cancel_agent 需要 id".into())
            } else if ctx.team.request_cancel(id) {
                Ok(format!("已请求取消后台任务 {id}（best-effort，它会在下个检查点收尾）。"))
            } else {
                Ok(format!("任务 {id} 不在运行中（可能已完成/已取消/不存在），无需取消。"))
            }
        } else {
            Err("只有主 agent 能取消后台任务".into())
        }
    } else if call.name == INSTRUCT_TOOL {
        if is_main {
            let id = input["id"].as_str().unwrap_or("").trim();
            let content = input["content"].as_str().unwrap_or("").trim();
            if id.is_empty() || content.is_empty() {
                Err("instruct_agent 需要 id 和 content".into())
            } else if !ctx.team.is_running(id) {
                Ok(format!("任务 {id} 不在运行中（可能已完成或不存在），指令未投递。用 team_status 查当前任务。"))
            } else {
                ctx.team.post_to_agent(id, content);
                Ok(format!("已把指令投给后台任务 {id}，它会在下一步抽到。"))
            }
        } else {
            Err("只有主 agent（协调者）能给后台任务下指令".into())
        }
    } else if call.name == REPORT_TOOL {
        match &ctx.team_task {
            Some(handle) => {
                let content = input["content"].as_str().unwrap_or("").trim();
                if content.is_empty() {
                    Err("content 不能为空".into())
                } else {
                    ctx.team.post_message(&handle.id, &handle.title, content);
                    Ok("已把消息投递给主 agent（协调者），它会在下一步看到。".into())
                }
            }
            None => Err("只有 spawn_team 派出的后台任务能用 report_to_coordinator".into()),
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

/// 异步派发后台团队任务：立即返回 task_id，不阻塞。后台 agent 完成后写注册表并推 TeamUpdate。
fn spawn_team(ctx: &AgentCtx, input: &Value, on_event: &EventSink<'_>) -> Result<String, String> {
    let tasks = input["tasks"].as_array().ok_or("缺少 tasks 参数")?;
    if tasks.is_empty() || tasks.len() > 8 {
        return Err("tasks 数量需在 1~8 之间".into());
    }

    // 全局并发上限：一次性算可用名额（防 TOCTOU），超额只派可用数、其余明确说明不派
    let available = MAX_CONCURRENT_TEAM.saturating_sub(ctx.team.running_count());
    if available == 0 {
        return Err(format!(
            "已有 {} 个后台任务在跑，达并发上限 {MAX_CONCURRENT_TEAM}，请等部分完成后再派。",
            ctx.team.running_count()
        ));
    }
    let requested = tasks.len();
    let dispatch_n = requested.min(available);

    let mut lines = Vec::new();
    for t in tasks.iter().take(dispatch_n) {
        let title = t["title"].as_str().ok_or("子任务缺少 title")?.to_string();
        let prompt = t["prompt"].as_str().ok_or("子任务缺少 prompt")?.to_string();
        let role_name = t["role"].as_str().filter(|r| !r.is_empty()).map(str::to_string);

        let id = ctx.team.next_id();
        ctx.team.start(&id, &title, role_name.clone());

        // 后台 agent 用独立 owned 上下文（'static），角色无效则退回通用
        let role = role_name
            .as_deref()
            .and_then(|n| crate::agents::resolve(ctx.workspace.as_deref(), n))
            .map(Arc::new);
        // 后台 agent 无交互审批通道：永远强制 Auto，角色的任何 permission 设置一律忽略，
        // 否则遇到写操作会永久阻塞在审批等待（危险操作仍会触发审批，背景任务里应避免）。
        let mut child = ctx.child_with_role(role);
        child.mode = AgentMode::Auto;
        // 赋予身份：据此用 report_to_coordinator 向主 agent 汇报
        child.team_task = Some(crate::agent::team::TeamTaskHandle {
            id: id.clone(),
            title: title.clone(),
        });
        let team = ctx.team.clone();
        let bg = ctx.bg_events.clone();
        let id_for_task = id.clone();
        let title_for_task = title.clone();

        tokio::spawn(async move {
            // 后台 agent 的内部事件不进主会话流（避免与主 agent 输出交错）
            let quiet = |_e: AgentEvent| {};
            // 墙钟兜底超时：超时丢弃 future（只停 agent 编排；已在跑的 shell 命令有自身 timeout）
            let run = loop_impl(
                &child,
                vec![HistoryItem::User(prompt)],
                &quiet,
                format!("team-{id_for_task}-"),
                false,
                None,
            );
            let result: Result<String, String> = match tokio::time::timeout(TASK_TIMEOUT, run).await {
                Ok(r) => r,
                Err(_) => Err(format!("任务超时（>{}s）", TASK_TIMEOUT.as_secs())),
            };
            // 完成推送（对齐 Claude Code / OpenClaw）：把"任务结束"投进协调者信箱，
            // 主 agent 下一步即可看到，无需主动轮询 team_status 才发现完事。
            // 被取消的任务 finish() 会收敛为 Cancelled，故先看 cancel 再定文案。
            let cancelled = team.is_cancel_requested(&id_for_task);
            let done_note = if cancelled {
                "⏹ 任务已取消。".to_string()
            } else {
                match &result {
                    Ok(r) => {
                        let brief: String = r.chars().take(280).collect();
                        format!("✅ 任务完成。摘要：{brief}")
                    }
                    Err(e) => format!("❌ 任务失败：{e}"),
                }
            };
            team.finish(&id_for_task, result);
            team.post_message(&id_for_task, &title_for_task, &done_note);
            // 刷新任务看板
            if let Some(emit) = &bg {
                emit(AgentEvent::TeamUpdate { tasks: team.snapshot() });
            }
        });

        let role_tag = role_name.as_deref().map(|r| format!(" [{r}]")).unwrap_or_default();
        lines.push(format!("- {id}: {title}{role_tag}"));
    }

    // 初始看板刷新
    on_event(AgentEvent::TeamUpdate { tasks: ctx.team.snapshot() });

    let overflow = if dispatch_n < requested {
        format!(
            "\n\n⚠️ 因并发上限 {MAX_CONCURRENT_TEAM}，本次只派了 {dispatch_n} 个，其余 {} 个未派——可在部分任务完成后重试。",
            requested - dispatch_n
        )
    } else {
        String::new()
    };

    Ok(format!(
        "已派发 {dispatch_n} 个后台任务，正在并行执行：\n{}{overflow}\n\n用 team_status 查询进度与结果（不必马上查，可以先做别的）。",
        lines.join("\n")
    ))
}

/// 查询后台团队任务状态/结果，并刷新看板
fn team_status(ctx: &AgentCtx, input: &Value, on_event: &EventSink<'_>) -> String {
    let records = match input["ids"].as_array() {
        Some(arr) => {
            let ids: Vec<String> =
                arr.iter().filter_map(|v| v.as_str().map(String::from)).collect();
            ctx.team.get(&ids)
        }
        None => ctx.team.snapshot(),
    };
    on_event(AgentEvent::TeamUpdate { tasks: ctx.team.snapshot() });

    if records.is_empty() {
        return "当前没有后台团队任务。".into();
    }
    let body = records
        .iter()
        .map(|r| {
            let status = match r.status {
                crate::agent::team::TaskStatus::Running => "运行中",
                crate::agent::team::TaskStatus::Done => "已完成",
                crate::agent::team::TaskStatus::Failed => "失败",
                crate::agent::team::TaskStatus::Cancelled => "已取消",
            };
            let role = r.role.as_deref().map(|x| format!(" [{x}]")).unwrap_or_default();
            match &r.result {
                Some(res) => format!("### {} {}{} — {}\n{}", r.id, r.title, role, status, res),
                None => format!("### {} {}{} — {}", r.id, r.title, role, status),
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let running = ctx.team.running_count();
    format!("团队任务（{running} 个运行中）：\n\n{body}")
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

    /// 给定角色名的离线 ctx（不读 key，仅用于不触网的执行期逻辑测试）
    fn ctx_with_role(workspace: Option<PathBuf>, role: &str) -> AgentCtx {
        let endpoint = registry::resolve("ark").unwrap();
        let role = crate::agents::resolve(workspace.as_deref(), role).map(Arc::new);
        assert!(role.is_some(), "角色应存在");
        AgentCtx {
            endpoint,
            api_key: String::new(),
            model: "doubao-seed-2.0-pro".into(),
            registry: Arc::new(ToolRegistry::builtin()),
            workspace,
            permissions: Arc::new(PermissionManager::default()),
            mode: AgentMode::Auto,
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            provider: "ark".into(),
            trace: None,
            todos: Arc::new(std::sync::Mutex::new(Vec::new())),
            app_data: None,
            session_id: None,
            pending: Arc::new(std::sync::Mutex::new(Vec::new())),
            role,
            team: Arc::new(crate::agent::team::TeamRegistry::default()),
            bg_events: None,
            team_task: None,
        }
    }

    /// 无 key 的离线 ctx（不触网，仅验证编排管线本身）
    fn offline_ctx() -> AgentCtx {
        let endpoint = registry::resolve("ark").unwrap();
        AgentCtx {
            endpoint,
            api_key: String::new(),
            model: "doubao-seed-2.0-pro".into(),
            registry: Arc::new(ToolRegistry::builtin()),
            workspace: None,
            permissions: Arc::new(PermissionManager::default()),
            mode: AgentMode::Auto,
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(true)), // 置 true：后台 loop 立刻收尾，不真跑
            provider: "ark".into(),
            trace: None,
            todos: Arc::new(std::sync::Mutex::new(Vec::new())),
            app_data: None,
            session_id: None,
            pending: Arc::new(std::sync::Mutex::new(Vec::new())),
            role: None,
            team: Arc::new(crate::agent::team::TeamRegistry::default()),
            bg_events: None,
            team_task: None,
        }
    }

    /// 功能验证：spawn_team 立即登记任务、返回 id、刷新看板（不阻塞、不依赖 LLM）
    #[tokio::test]
    async fn spawn_team_registers_and_returns_ids() {
        let ctx = offline_ctx();
        let board_updates = Arc::new(std::sync::Mutex::new(0usize));
        let bu = board_updates.clone();
        let sink = move |e: AgentEvent| {
            if let AgentEvent::TeamUpdate { tasks } = e {
                let _ = tasks;
                *bu.lock().unwrap() += 1;
            }
        };
        let input = serde_json::json!({"tasks":[
            {"title":"查A","prompt":"调查 A"},
            {"title":"审B","prompt":"审查 B","role":"review"}
        ]});
        let out = spawn_team(&ctx, &input, &sink).unwrap();
        assert!(out.contains("t0") && out.contains("t1"), "应返回 task_id：{out}");

        let snap = ctx.team.snapshot();
        assert_eq!(snap.len(), 2, "两个任务应立即登记");
        assert_eq!(snap[0].title, "查A");
        assert_eq!(snap[1].role.as_deref(), Some("review"));
        assert!(*board_updates.lock().unwrap() >= 1, "应至少刷新一次看板");

        // team_status 能查到这些任务
        let status = team_status(&ctx, &serde_json::json!({}), &|_e| {});
        assert!(status.contains("查A") && status.contains("审B"));

        // 完成推送：后台任务收尾（cancel=true 会立即 bail）后，应把"任务结束"投进协调者信箱
        let mut done_msgs = Vec::new();
        for _ in 0..50 {
            done_msgs.extend(ctx.team.drain_inbox());
            if done_msgs.len() >= 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(done_msgs.len(), 2, "两个任务都应推送完成消息");
        assert!(done_msgs.iter().all(|m| m.content.contains("任务完成") || m.content.contains("任务失败")));
    }

    /// 端到端 live 验证：spawn_team 真的在后台跑通一个 agent 并回填结果。
    /// 用多线程 runtime——后台 agent 的阻塞 I/O 不会饿死轮询计时器。
    /// cargo test live_team -- --ignored --nocapture
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore]
    async fn live_team_runs_background_task() {
        let ctx = ark_ctx(None);
        let input = serde_json::json!({"tasks":[
            {"title":"算术","prompt":"1+1 等于几？只回答阿拉伯数字，不要任何解释。"}
        ]});
        let dispatched = spawn_team(&ctx, &input, &|_e| {}).unwrap();
        println!(">> {dispatched}");

        // 轮询直到后台任务收尾（最多 ~30s）
        for _ in 0..60 {
            if ctx.team.running_count() == 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        let snap = ctx.team.snapshot();
        println!(">> 任务结果: {:?}", snap);
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].status, crate::agent::team::TaskStatus::Done, "后台任务应完成");
        assert!(snap[0].result.as_ref().unwrap().contains('2'), "结果应包含 2");
    }

    /// 功能验证：后台 agent 的 report_to_coordinator 真的把消息投进协调者信箱
    #[tokio::test]
    async fn report_tool_posts_to_inbox() {
        let mut ctx = offline_ctx();
        ctx.team_task = Some(crate::agent::team::TeamTaskHandle {
            id: "t0".into(),
            title: "查依赖".into(),
        });
        let call = ToolCall {
            id: "r1".into(),
            name: "report_to_coordinator".into(),
            arguments: "{\"content\":\"发现 X 库已废弃，建议换 Y\"}".into(),
        };
        let read_files = Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));
        let result = execute_call(&ctx, "", &call, &|_e| {}, None, false, &read_files).await;
        match result {
            HistoryItem::ToolResult { is_error, .. } => assert!(!is_error, "汇报应成功"),
            _ => panic!(),
        }
        let msgs = ctx.team.drain_inbox();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].from_id, "t0");
        assert_eq!(msgs[0].from_title, "查依赖");
        assert!(msgs[0].content.contains("废弃"));
    }

    /// 非团队 agent 不能用 report_to_coordinator
    #[tokio::test]
    async fn report_tool_rejected_without_team_task() {
        let ctx = offline_ctx(); // team_task = None
        let call = ToolCall {
            id: "r1".into(),
            name: "report_to_coordinator".into(),
            arguments: "{\"content\":\"x\"}".into(),
        };
        let read_files = Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));
        let result = execute_call(&ctx, "", &call, &|_e| {}, None, false, &read_files).await;
        match result {
            HistoryItem::ToolResult { is_error, .. } => assert!(is_error, "非团队 agent 应被拒"),
            _ => panic!(),
        }
        assert!(ctx.team.drain_inbox().is_empty());
    }

    /// 功能验证：cancel_agent 标记取消，且后台 child 的 is_cancelled 立即为真
    #[tokio::test]
    async fn cancel_agent_marks_and_propagates() {
        let ctx = offline_ctx();
        let id = ctx.team.next_id();
        ctx.team.start(&id, "长任务", None);

        let call = ToolCall {
            id: "c1".into(),
            name: "cancel_agent".into(),
            arguments: format!("{{\"id\":\"{id}\"}}"),
        };
        let read_files = Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));
        let r = execute_call(&ctx, "", &call, &|_e| {}, None, true, &read_files).await;
        match r {
            HistoryItem::ToolResult { is_error, content, .. } => {
                assert!(!is_error);
                assert!(content.contains("已请求取消"));
            }
            _ => panic!(),
        }
        assert!(ctx.team.is_cancel_requested(&id));

        // 后台 child（带该任务身份）应感知取消
        let mut child = ctx.child_with_role(None);
        child.team_task = Some(crate::agent::team::TeamTaskHandle { id: id.clone(), title: "长任务".into() });
        assert!(is_cancelled(&child), "被取消的后台任务 is_cancelled 应为真");
    }

    /// 功能验证：spawn_team 尊重全局并发上限（满则拒绝；超额只派可用数并说明）
    #[tokio::test]
    async fn spawn_team_respects_concurrency_cap() {
        let ctx = offline_ctx();
        // 占满并发：手动登记 MAX_CONCURRENT_TEAM 个运行中任务
        for _ in 0..MAX_CONCURRENT_TEAM {
            let id = ctx.team.next_id();
            ctx.team.start(&id, "占位", None);
        }
        // 再派应被拒
        let full = spawn_team(&ctx, &serde_json::json!({"tasks":[{"title":"x","prompt":"y"}]}), &|_e| {});
        assert!(full.is_err(), "满并发应拒绝");
        assert!(full.unwrap_err().contains("并发上限"));
    }

    /// 功能验证：主 agent 的 instruct_agent 把指令投进运行中任务的信箱；任务不在跑则不投
    #[tokio::test]
    async fn instruct_agent_posts_to_running_task_mailbox() {
        let ctx = offline_ctx();
        let id = ctx.team.next_id();
        ctx.team.start(&id, "开发X", Some("dev".into()));
        let read_files = Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));

        // 运行中 → 指令进信箱
        let call = ToolCall {
            id: "i1".into(),
            name: "instruct_agent".into(),
            arguments: format!("{{\"id\":\"{id}\",\"content\":\"改用方案B\"}}"),
        };
        let r = execute_call(&ctx, "", &call, &|_e| {}, None, true, &read_files).await;
        match r {
            HistoryItem::ToolResult { is_error, .. } => assert!(!is_error),
            _ => panic!(),
        }
        assert_eq!(ctx.team.drain_agent_mailbox(&id), vec!["改用方案B".to_string()]);

        // 任务已完成 → 不投递，明确提示
        ctx.team.finish(&id, Ok("done".into()));
        let call2 = ToolCall {
            id: "i2".into(),
            name: "instruct_agent".into(),
            arguments: format!("{{\"id\":\"{id}\",\"content\":\"再改\"}}"),
        };
        let r2 = execute_call(&ctx, "", &call2, &|_e| {}, None, true, &read_files).await;
        match r2 {
            HistoryItem::ToolResult { content, .. } => assert!(content.contains("不在运行")),
            _ => panic!(),
        }
        assert!(ctx.team.drain_agent_mailbox(&id).is_empty());
    }

    /// 功能验证：review 角色在执行期真的拦截 edit_file（不止是 spec 过滤）
    #[tokio::test]
    async fn review_role_blocks_edit_at_execution() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "原始内容\n").unwrap();
        let ctx = ctx_with_role(Some(dir.path().to_path_buf()), "review");

        let call = ToolCall {
            id: "e1".into(),
            name: "edit_file".into(),
            arguments: "{\"path\":\"f.txt\",\"old_string\":\"原始内容\",\"new_string\":\"被篡改\"}".into(),
        };
        let read_files = Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));
        let sink = |_e: AgentEvent| {};
        let result = execute_call(&ctx, "", &call, &sink, None, true, &read_files).await;

        match result {
            HistoryItem::ToolResult { is_error, content, .. } => {
                assert!(is_error, "review 角色调 edit_file 应被拒绝");
                assert!(content.contains("review"), "拒绝信息应点明当前角色：{content}");
            }
            _ => panic!("应返回 ToolResult"),
        }
        // 关键：文件内容没被改动，证明拦截发生在执行之前
        assert_eq!(std::fs::read_to_string(dir.path().join("f.txt")).unwrap(), "原始内容\n");
    }

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
            team: Arc::new(crate::agent::team::TeamRegistry::default()),
            bg_events: None,
            team_task: None,
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
