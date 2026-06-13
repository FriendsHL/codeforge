# 功能验证规范（codeForge）

> 取自 skillForge `.codex/rules/verification-before-completion.md` + `pipeline.md`，
> 按 codeForge（Tauri + Rust + React）的约束落地。目的：不再"编译过=完成"。

## 铁律

**没有「本回合新鲜的验证证据」，不准声称完成 / 修好 / 通过 / 可用。**

build 成功只证明「能编译」，不证明「能工作」。尤其前端：build 绿 ≠ 页面对。

## 收尾前的闸门

每次声称做完前：
1. 指出能证明它的命令/检查；
2. **本回合**重新跑一遍；
3. 读 stdout/stderr/退出码/失败数；
4. 确认输出支持结论；
5. 带证据汇报。

## codeForge 证据表

| 改动类型 | 必须的证据 |
|---|---|
| Rust 后端逻辑 | `cargo test --lib` 全绿；新增逻辑要有对应单测 |
| Agent / 工具 / 角色行为 | **功能测试**：直接调 `execute_call`/`run_agent_loop` 断言行为（如 `review_role_blocks_edit_at_execution` 证明角色闸门真拦截、文件真没改）；端到端用 `cargo test live_* -- --ignored`（真实 Ark API） |
| 前端纯逻辑 | `npx vitest run` 全绿；store/纯函数要有 DOM/文本级断言（testing-library + jsdom），不止纯函数 |
| 前端可用性（Tauri webview 视觉/交互） | 自动验不了（终端无屏幕录制权限）→ 给用户**烟雾测试清单**逐条确认，或单独搭 tauri-driver e2e |
| Bug 修复 | 用原始复现步骤证明症状消失；能做到就红→绿（没修时挂、修后过） |
| 子 agent 完成 | 主流程亲自 `git diff` + 抽查改动文件，不轻信 agent 自报 |

## 分级（按风险定投入）

- **Solo**：单行/注释/文档/常量/机械重命名/已被强单测锁住的纯函数 —— 直接改 + 跑测试。
- **Mid**（默认）：普通 bug、可见 UI 行为、加字段不改 schema、新端点不改 schema —— 方案(可省)→开发→一轮 review→验证。
- **Full**（红线）：核心文件（`agent/loop_.rs`、`llm/**`、`session/mod.rs`、`commands/chat.rs`、`ChatView.tsx`）、配对协议（tool_use/tool_result）、跨 3 模块特性、踩过的坑 —— 方案→开发→对抗式 review(≤2 轮)→主流程亲自终验。

## 我（助手）做功能验证的三层

1. **后端/agent**：`cargo test`（含直调 execute_call/loop 的功能测试）+ 按需 live 集成测试（真实 Ark）。✅ 能自动跑。
2. **前端逻辑/DOM**：vitest + testing-library + jsdom，带 DOM/文本断言。✅ 能自动跑。
3. **Tauri webview 视觉**：❌ 无屏幕权限 → 输出烟雾测试清单给用户确认。

> 每次交付要按这表给证据；做不到的层要**明说"这层没自动验，需你确认"**，不能含糊带过。
