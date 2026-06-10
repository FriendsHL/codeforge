# codeForge 架构设计

> macOS 桌面端 coding agent。Tauri 2 + React/TS 前端 + Rust 后端。
> 本文档是项目的总设计图，开发顺序见 [ROADMAP.md](./ROADMAP.md)。

## 1. 总体定位

codeForge 是一个**自研 agent loop** 的桌面 coding agent（类比 Claude Code / Cursor 的 agent 模式，
但以独立桌面 app 形态存在）：用户选择一个本地项目目录，与 agent 对话，
agent 通过工具（读写文件、搜索、执行命令）自主完成编码任务，
所有危险操作经过用户审批，文件改动以 diff 形式呈现。

> 备选方案曾考虑"包一层现有 CLI（claude-code/codex）做纯 GUI"，
> 否决：核心价值和学习目标都在 agent loop 本身，壳没有积累。

## 2. 分层架构

```
┌─────────────────────────────────────────────────────┐
│  React 前端 (src/)                                   │
│  会话流 UI · diff 审查 · 内嵌终端 · 设置              │
├──────────────── Tauri IPC ──────────────────────────┤
│  commands/  ← IPC 入口层（类比 Spring @Controller）   │
├─────────────────────────────────────────────────────┤
│  Rust 核心 (src-tauri/src/)                          │
│  agent/   核心循环（类比 @Service）                   │
│  llm/     模型客户端（Provider trait，可换模型）       │
│  tools/   工具集（Tool trait，策略模式）              │
│  security/ 权限审批   session/ 持久化                 │
│  pty/     终端进程    config/ 配置+密钥               │
└─────────────────────────────────────────────────────┘
```

**职责边界**：前端只做渲染和交互，不含任何 agent 逻辑；
Rust 端不关心 UI，只通过事件流向前端推送状态。
类比前后端分离：IPC `invoke` ≈ REST 调用，Tauri `Channel` 事件 ≈ WebSocket 推送。

## 3. Rust 端模块设计

```
src-tauri/src/
├── main.rs / lib.rs        # 入口、插件注册、状态注入
├── commands/               # Tauri command（薄，只做参数转换+调度）
│   ├── chat.rs             #   send_message / cancel / approve_permission
│   ├── workspace.rs        #   open_project / list_sessions
│   │                       #   read_dir_tree（文件树，懒加载子目录，遵循 .gitignore）
│   │                       #   read_file_preview（文件内容预览）
│   └── settings.rs         #   get/set 配置、API key
├── agent/
│   ├── loop_.rs            # 核心循环：组装上下文 → 调 LLM → 解析工具调用
│   │                       #   → 审批 → 执行 → 结果回填 → 直到无工具调用
│   ├── prompt.rs           # system prompt 模板
│   └── events.rs           # AgentEvent 枚举（流式推给前端的统一事件协议）
├── llm/
│   ├── provider.rs         # trait Provider { stream_chat(...) }
│   ├── anthropic.rs        # 首发：Anthropic Messages API（SSE 流式 + tool use）
│   └── types.rs            # Message / ToolCall / Usage 等领域类型
├── tools/
│   ├── registry.rs         # trait Tool { name/schema/run } + 动态注册表
│   │                       #   设计为多来源：v1 只有内置工具，
│   │                       #   v2 接入 MCP client 和 skill 提供的工具，不改 loop
│   ├── fs.rs               # read_file / write_file / edit_file / list_dir
│   ├── search.rs           # glob + 内容搜索（用 ripgrep 的库 grep-searcher）
│   └── bash.rs             # 命令执行（经 pty/ 模块）
├── security/
│   └── policy.rs           # 操作分级：只读直通 / 写文件·执行命令需审批
│                           # 审批经事件推到前端，await 用户决定（oneshot channel）
├── pty/
│   └── manager.rs          # portable-pty 封装：spawn / 流式输出 / kill
├── session/
│   ├── store.rs            # SQLite (rusqlite)：sessions / messages / tool_calls 表
│   └── models.rs
└── config/
    └── mod.rs              # 应用配置（JSON 文件）+ API key（macOS Keychain，keyring crate）
```

**关键依赖**：`tokio`（异步运行时）、`reqwest` + `eventsource-stream`（SSE）、
`portable-pty`、`rusqlite`、`keyring`、`similar`（diff 计算）、
`grep-searcher`/`globset`/`ignore`（搜索三件套，ripgrep 同源）。

## 4. 前端模块设计

```
src/
├── lib/
│   ├── ipc.ts              # 所有 invoke 的类型化封装（唯一与 Tauri 接触的文件）
│   └── events.ts           # AgentEvent 监听与分发
├── stores/                 # zustand：chatStore / sessionStore / settingsStore
├── components/
│   ├── chat/               # 消息流（流式 markdown）、输入框、工具调用卡片
│   ├── explorer/           # 文件树面板（antd Tree 懒加载）+ 文件预览（只读、语法高亮）
│   ├── diff/               # diff 审查视图（approve / reject）
│   ├── terminal/           # xterm.js 内嵌终端（只读回显 agent 执行过程）
│   └── settings/           # API key、模型选择、权限策略
└── App.tsx                 # 布局：左侧 会话列表+文件树 / 主区会话 / 可折叠终端
                            # agent 正在读写的文件在树中高亮；改动后自动刷新（notify watcher）
```

**选型**：antd（沿用 skillForge 经验）、zustand（轻量状态）、
react-markdown + Shiki（代码高亮）、xterm.js（终端渲染）。

## 5. 核心数据流

一轮 agent 任务（前端视角全程只看事件流）：

```
用户发消息
  → invoke("send_message", { sessionId, text, channel })
  → Rust: agent loop 启动 tokio task，经 Channel 持续推送 AgentEvent：
      TextDelta        流式文本（打字机效果）
      ToolCallStart    agent 决定调用某工具（UI 显示工具卡片）
      PermissionAsk    需要审批（UI 弹出 diff / 命令确认，loop 挂起等待）
      ToolCallEnd      工具结果（卡片折叠显示）
      TurnEnd          本轮结束（含 token 用量）
  → 用户审批走 invoke("approve_permission", { requestId, decision })
```

中断：`invoke("cancel")` → 触发 `CancellationToken`，loop 在安全点退出。

## 6. 安全与权限

- 操作三级：**只读**（read/search，直通）、**写**（write/edit，默认审批，展示 diff）、
  **执行**（bash，默认审批，展示命令）。
- 会话级"本次全部允许"开关；路径越界保护（工具只能访问选定项目目录内）。
- API key 存 macOS Keychain，绝不落盘明文、绝不进前端。

## 7. v2 方向（v1 刻意不做，但已留好接缝）

| 能力 | 形态 | 预留的接缝 |
|---|---|---|
| 浏览器 | ① web_fetch/web_search 工具（agent 查文档）② 内嵌预览面板（看 localhost） | 工具走 registry，零改动 |
| Skill | 按需加载的指令包（SKILL.md + 脚本），可对接 skillForge 技能库 | prompt.rs 的 system prompt 做成分段组装 |
| MCP | MCP client（官方 rmcp SDK），接入整个 MCP 工具生态 | registry 多来源设计 |

其余暂不做：多 provider（trait 已留口子）、子 agent、checkpoint/回滚、跨平台（Win/Linux）、自动更新。
