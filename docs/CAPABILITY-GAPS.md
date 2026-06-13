# codeForge 能力缺口与需求规划

> 2026-06-13 编制。基于 `research-docs`(agent-harness-wiki / claude code·opencode·openclaw·Hermes 源码研究 / OpenClaw 记忆系统 / Anthropic 博客 / OpenAI Workspace Agents)对 13+ 成熟 harness 的拆解,对照 codeForge 现状梳理。
>
> **目标定位**:能编码 + 能调研的强大工具系统,**以编码为主**。

## 现状盘点(已具备)

- agent loop(串行工具)、并行子 agent(spawn_subagents,深度 1)
- 工具集:read/list/glob/grep · git_status/diff/log · write/edit · bash · web_fetch(→Markdown)/web_search(Tavily+DDG) · todo_write · browser_open · MCP 动态工具
- 上下文:轮内修剪(单结果中段截断+总预算保头尾)、跨轮 7 段式摘要、真实 token 计量、Anthropic prompt cache(system+last)
- 安全:审批 + 危险命令检测 + checkpoint 文件回滚
- 可观测:OTel 风格 .jsonl trace + `/trace` 聚合
- 体验:三栏可拖拽、深色、@文件引用、追加对话(steering)、停止、思考卡片、快捷命令、原生浏览器面板

## 核心缺口(按"编码优先 + 调研为辅"的目标排序)

### 🔴 P0 — 留存与差异化刚需

**1. 记忆系统(最大缺口)** — 现在完全无记忆,只有会话历史存储。Claude Code 有 CLAUDE.md/Memory,用户会立刻觉得"它记不住我"。
- 参考:OpenClaw 三阶段(Light 摄入 / Deep 六信号晋升 / 梦境叙述)、Claude Code 3 路召回(MEMORY.md 索引→小模型 sideQuery→Grep/Read)+ 4 路写入。
- 分阶段(详见 research agent 方案):
  - **阶段1**:`~/.codeforge/memory/` + SQLite,`memory_search` 工具(BM25+embedding),agent 回合结束自动抽"决策/发现";项目级 `.codeforge/MEMORY.md` 注入 system prompt
  - **阶段2**:Deep 六信号评分自动晋升到 MEMORY.md
  - **阶段3**:启动 prefetch 注入 + 后台提炼
- 战略价值:这是接 **skillForge 飞轮**的接口(codeForge 轨迹/记忆 → skillForge 技能进化 → 回流)。

**2. 调研能力升级** — 现仅 web_search/web_fetch 两个基础工具,够"查",不够"调研"。
- 缺:多源结果聚合去重、**引用溯源**(每条结论挂来源 URL)、**Generator-Verifier 自验证**(一个 agent 出结论、另一个反驳,最多 N 轮)、结构化报告生成。
- 参考:Anthropic 多 agent 协调(Generator-Verifier→Orchestrator→Message Bus)、Dynamic Workflows 的"Verify 阶段"、OpenAI Workspace 5 模式(Briefing/Triage/Analysis/Content/Planning)。
- 形态:不是"先调研后编码",而是"边调研边编码"——调研发现在前,本地实现在后,失败再搜反驳理由。可用子 agent 并行多角度调研 + 主 agent 综合。

**3. 可靠性硬化** — 用户最烦"agent 跑一半静默失败"。
- 错误类型化:`Result<_, String>` 全改 `enum AgentError`(网络/鉴权/取消/工具/限流),消灭 `CANCELLED_ERR` 魔法字符串。
- panic 审查:229 处 unwrap/expect(多为 `lock().unwrap()`),关键路径换成优雅降级。
- 前端零测试:2800 行 TS 没有一个测试,chatStore 事件处理该补。

### 🟡 P1 — coding 深度 + 上下文进阶

**4. LSP 一等集成** — 从"会读代码"到"理解代码"。
- 参考:opencode LSP first-class(diagnostic/hover/definition/references 喂给 Edit/Read/Grep)。
- 给工具加语义:edit 前看 diagnostic、跳转定义、找引用。对 coding-first 工具是硬通货。

**5. 上下文工程进阶** — 现在压缩是"有损摘要"一条路,成熟做法是分层。
- 参考 Claude Code 5 级流水线:toolResultBudget(已做)→ snip 截断 → **microcompact(cache_edits 无损,零 LLM)** → collapse 投影 → autocompact。
- **compact boundary + resume 链**:压缩后插浮标,可沿边界续命且老 transcript 可查。
- Read-before-Edit 闸门:同一文件 read 过才允许 edit(防"没读就改")。

**6. 工具并行执行** — 只读工具(read/grep/glob)现在串行白等。
- 参考 Hermes/Claude Code:streaming tool executor + 路径并发安全检查(读并行、同路径写串行)。

### 🟢 P2 — 分发与自进化

**7. 分发基础设施** — 签名/公证(等 Apple Developer 账号)+ tauri-updater 自动更新(现在手动替换 /Applications,真实用户无法升级)。

**8. 自进化** — session 失败 → 自动提炼 prompt/rule/skill 修订建议 + 人审落地。参考 Claude Code `/insights`、SkillForge `/evolve`。这是 skillForge 飞轮的另一半。

**9. 终端+浏览器面板整合** — 查看器区做成多 tab(文件/diff/终端/浏览器同处一栏),消除终端浮层。

## 明确"不做"(研究里的反面教训)

- **Code Mode(V8 isolate 执行 LLM 写的代码)** — Codex 独家,工程债极高,不值。
- **自己接 100+ provider** — 用 SDK 抽象即可,现有 registry 够用。
- **重型知识图谱记忆** — ICLR'26 GraphRAG-Benchmark 警示 KG 未必优于混合检索;记忆走"文件+SQLite+FTS5/向量"轻量路线。

## 建议迭代顺序(ROI)

1. **记忆系统阶段1**(留存刚需 + skillForge 接口)
2. **可靠性硬化**(错误类型化 + panic 审查 + 前端关键测试)
3. **调研能力升级**(引用溯源 + Generator-Verifier + 多源综合)
4. **LSP 集成** / **工具并行**(coding 深度)
5. **签名 + 自动更新**(分发)
6. 上下文进阶(microcompact/boundary)、自进化、面板整合

> 一句话战略:不在通用 coding 能力上正面打 Claude Code/Codex(打不赢),赢点是 **"模型自由 + 本地隐私 + GUI 可视化 + 记忆/调研 + skillForge 飞轮"** 的组合。
