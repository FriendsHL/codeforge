# 团队编排加固方案（4 项）

> 目标：把 createTeam 异步编排补齐到 Claude Code / OpenClaw 的健壮度，但保持 codeForge 的精简取舍。
> 现状：`spawn_team`(异步派发) + `team_status`(轮询) + 协调者双向信箱(report/instruct) + 完成推送。
> 注册表 `TeamRegistry` 在 `AppState`（内存态）。后台 agent 强制 Auto 模式。

---

## 1. 全局并发上限

**问题**：现在 `spawn_team` 每次 1~8，但跨多次调用没有全局上限，可能堆出几十个后台 agent。

**方案**（对齐 OpenClaw 4/8/5，取精简版）：
- 常量 `MAX_CONCURRENT_TEAM = 8`（全局同时运行的后台任务上限）。
- `spawn_team` 派发前：`available = MAX - team.running_count()`。
  - `available <= 0` → 整个调用拒绝，返回「已有 N 个后台任务在跑，达并发上限，请等部分完成后再派」。
  - `requested > available` → 只派前 `available` 个，返回里**明确说明**「因并发上限只派了 X 个，其余 Y 个未派，可稍后重试」（不静默丢）。
- 不做排队调度器（队列+空位调度复杂度高，收益小）。reject-excess 简单可预测。

**取舍**：选 reject-excess 而非 queue。理由：队列需要"空位释放时自动启动 pending"的调度逻辑 + 状态机，对桌面单机场景过重。

**测试**：running_count 达上限时 spawn_team 拒绝；部分超额时只派可用数且消息说明。

---

## 2. per-task 取消 / 超时

**问题**：现在只有全局 `cancel`（停止按钮停所有）。无法单独停一个后台任务；无超时，卡住的任务永远 Running。

**方案**：
### 2a. per-task 取消
- `TeamRegistry` 加 `cancelled: Mutex<HashSet<String>>`。
- `cancel_task(id)` / `is_task_cancelled(id)`。
- 后台 child 的取消判断扩展：`is_cancelled(ctx)` 对团队 child 额外检查 `ctx.team.is_task_cancelled(handle.id)`（仍保留全局 `ctx.cancel`，全局停止照旧停所有）。
- 新工具 `cancel_agent(id)`（主 agent 用，loop 内特判）：标记取消，任务下一个检查点收尾，finish 为 `Failed("已被协调者取消")`。

### 2b. 超时
- 常量 `TASK_TIMEOUT = Duration::from_secs(300)`（可被角色 maxTurns 间接影响，但超时是墙钟兜底）。
- `spawn_team` 后台块用 `tokio::time::timeout(TASK_TIMEOUT, loop_impl(...))` 包裹。
  - 超时 → `finish(Failed("任务超时（>300s）"))` + 完成推送「❌ 超时」。
- 超时丢弃 future 在 await 点安全（loop_impl 在 LLM/工具 await 处可被取消）。

**取舍**：超时用墙钟 `tokio::time::timeout` 而非每轮 deadline 检查——更简单、对卡在单次 LLM 调用的情况也有效。

**测试**：cancel_task 后 is_task_cancelled 为真；cancel_agent 对运行中任务标记取消（功能级，execute_call）；超时分支用极小 timeout + 永跑 mock 验证 finish=Failed（或单测 is_task_cancelled 逻辑，超时集成留 ignored）。

---

## 3. 任务持久化（含诚实的范围界定）

**关键认知**：codeForge 是**无 daemon 的桌面 app**。app 关闭 → tokio 后台任务**直接死**，无法 resume。所以"持久化正在跑的任务并恢复"**不适用**（Claude Code 同样内存态、不持久；OpenClaw 持久是因为有 server 能恢复）。

**方案**（持久化仅用于**历史/审计**，不做 resume）：
- `sessions` 同库新增表 `team_tasks(id TEXT PK, session_id INTEGER, title, role, status, result, created_at)`。
- `spawn_team` 登记时 insert（status=running）；`finish` 时 update（status+result）。
  - 写库走 `AppState` 里的 `SessionStore`（需让 TeamRegistry 能拿到 store，或在 finish 回调里写）。
- **启动时**：把所有 `status='running'` 的旧记录改成 `'interrupted'`（它们的进程已死）。
- `team_status` 仍以**内存注册表**为实时源；持久表用于"重启后还能看到上次跑过什么"（可选地在启动时把历史载入注册表，标记 interrupted）。

**待 review 决断的点**：这项**收益最存疑**。无 resume 的前提下，持久化只换来"重启后能看历史"。是否值得引入新表 + TeamRegistry↔SessionStore 的耦合？
- 备选 A：按上述做轻量历史持久化。
- 备选 B：**不做持久化**，只在内存；重启清空（与 Claude Code 一致）。文档写明"后台任务不跨重启"。
- 倾向：**B（不做）** 或 最小化 A。请 reviewer 重点挑这一项。

---

## 4. 角色配置更丰富

**现状**：`AgentRole { name, description, system_prompt, tools }`。

**方案**：frontmatter 增加可选字段，解析进 AgentRole：
- `model`：**裸 model id**，仅在**当前会话的同一 provider 内**覆盖 `ctx.model`（避免跨 provider 的 endpoint/key 重解析复杂度）。跨 provider 的需求暂不支持，文档写明。
- `maxTurns`：覆盖该角色 agent 的 `MAX_ITERATIONS`（loop 用 `ctx.role.max_turns.unwrap_or(MAX_ITERATIONS)`）。
- `permissionMode`（ask/auto/plan）：
  - 作用于**子 agent / team child**（作为它的 mode）；对 team child 覆盖"强制 Auto"。
  - 对**会话驱动**（用户在顶栏选的角色）：UI 选的 mode 是显式用户意图 → **UI 优先**，忽略角色的 permissionMode。
- `isolation`（worktree/remote）：**不做**。codeForge 无 worktree/remote 基建，引入成本高。

**数据流**：
- AgentRole 加 `model: Option<String>`, `max_turns: Option<usize>`, `permission_mode: Option<AgentMode>`。
- save_role 写这些字段；parse 读。
- loop_body：`let max_iter = ctx.role.and_then(|r| r.max_turns).unwrap_or(MAX_ITERATIONS);` 循环用之。
- model 覆盖：在 build ctx 时（会话驱动 in chat.rs，subagent in child_with_role/spawn_team）按规则套用。

**测试**：parse 读出 model/maxTurns/permissionMode；maxTurns 真的改了循环上限（可注入小值跑离线 loop 验证迭代次数受限）；save_role 往返这些字段。

---

## 实施顺序与风险
1. 角色配置（4）——独立、低风险，先做。
2. 全局并发上限（1）——小，TeamRegistry + spawn_team 改。
3. per-task 取消/超时（2）——中，碰 is_cancelled + 新工具 + spawn 块。
4. 持久化（3）——**看 review 结论**，可能不做或最小化。

每项按验证纪律给功能测试证据；Tauri 视觉层给烟雾清单。

## ✅ Review 结论与修订（subagent 对抗式 review 后，采纳）

review 判定：不能直接开发，需修 4 blocker。修订如下：

1. **持久化 → 砍掉（选 B，不做）**。理由：① SessionStore 在独立的 `SessionState` 而非 `AppState`，后台 tokio task 拿不到句柄，耦合比预想深；② 任务完成摘要已通过 `report_to_coordinator` 推进主 agent 会话历史，而**会话历史本就持久化**，再建 `team_tasks` 表是冗余；③ 无 daemon 不能 resume。文档写明"后台任务不跨重启"。
2. **per-task 取消信号独立化（不共享全局 cancel）**。现状 bug：后台 child 克隆 `AppState.cancel`，而新回合 `store(false)` 会抹掉停止标志。改为：
   - `TaskRecord` 加 `cancel_requested: bool`（不另开 HashSet，随终态记录自然清理，省一把锁）。
   - `spawn_team` 给后台 child 一个**全新独立** cancel flag（不共享全局）。
   - `is_cancelled(ctx)`：团队 child 额外检查 `ctx.team.is_cancel_requested(task_id)`。
   - 全局停止：`stop_generation` 除置全局 cancel 外，调 `team.cancel_all_running()`（把所有 Running 标记取消）。
   - `cancel_agent(id)` 工具：标记单个。
   - **诚实写明**：cancel 是 best-effort。工具在 `spawn_blocking` 上跑、`pty` 不收 cancel，正卡在 `cargo build`(可达 600s) 的任务最多要等它返回才到下个检查点收尾。
3. **超时设为粗粒度兜底，避开孤儿进程**。`TASK_TIMEOUT = 1800s(30min)`——**大于** bash 自身 max timeout(600s)，所以 bash 会先自我超时返回，不会出现"task 超时标 Failed 但 bash 子进程还在改磁盘"的孤儿。文档写明：超时只是"卡死任务"的最后兜底，不停已在跑的 shell 命令（它有自己的 timeout）。
4. **permissionMode 砍掉（连同 isolation 一起 defer）**。review 指出它是审批老坑的磁铁、且对会话驱动/同步子agent/team child 的优先级规则混乱、收益低。角色配置**本轮只加 `model` + `maxTurns`**。后台 team child 永远强制 Auto，不受角色影响。
5. **并发上限：一次性算 available**（防 TOCTOU），不在循环里反复查。
6. **model 覆盖**：仅同 provider 裸 id；解析/覆盖时 log warn；调用失败信息原样进 `finish(Failed)`；文档写明 provider 约束。
7. **TaskRegistry 加终态清理上限**（现状已有的小泄漏）：保留最近 N=50 个任务，超出淘汰最旧的终态记录。
8. **maxTurns**：循环上限 + 报错文案(loop_.rs 内 MAX_ITERATIONS 两处)都改成变量。

修订后实施顺序：先修 #2 的全局 cancel 根因 → 角色 model/maxTurns → 并发上限 → per-task 取消/超时 → 注册表清理。持久化不做。

## （原）请 reviewer 重点挑的点
- 并发上限：reject-excess vs queue，选对了吗？
- 取消语义：per-task 标记 + 全局 cancel 并存，有没有竞态/遗漏？
- **持久化到底做不做**（最存疑）？
- 角色 model 覆盖限定"同 provider"是否合理？permissionMode 对会话驱动"UI 优先"对不对？
- 有没有过度设计 / 与 codeForge 精简哲学冲突的地方？
