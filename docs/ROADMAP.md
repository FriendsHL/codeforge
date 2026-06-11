# codeForge 开发路线图

原则：**每个里程碑结束时 app 都能跑、都有肉眼可见的新能力**（纵向切片，不做横向铺设）。
M1 是最薄的端到端打通，后面每步在其上叠加。

## M1 — 能聊天 ✅（v0.1，2026-06-11 验收通过）

最小闭环，打通 前端 → IPC → Rust → Anthropic API → 流式事件 → UI 全链路。

1. 前端布局骨架：会话区 + 输入框 + 设置页（antd）
2. `config/`：API key 存取（keyring → macOS Keychain）+ 设置页对接
3. `llm/anthropic.rs`：Messages API 流式客户端（reqwest + SSE）
4. `commands/chat.rs`：send_message + Channel 推 TextDelta 事件
5. 前端流式渲染（react-markdown，打字机效果）

**验收**：填入 API key，发"你好"，看到流式回复。

## M2 — 能看代码 ✅（v0.2，2026-06-11 验收通过）

引入 agent loop 和只读工具，这是项目的心脏。

1. `tools/registry.rs`：Tool trait + JSON Schema 注册表
2. 只读工具：read_file / list_dir / glob / grep
3. `agent/loop_.rs`：完整 tool-use 循环（LLM 返回 tool_use → 执行 → 回填 → 再调 LLM）
4. 工作区：打开项目目录（文件夹选择器），工具限定在目录内
5. **文件树面板**：左侧目录树（懒加载、遵循 .gitignore），点击文件只读预览（语法高亮）；
   agent 正在读的文件在树中高亮
6. UI：工具调用卡片（显示 agent 正在读哪个文件、搜什么）

**验收**：打开 skillForge 目录，左侧能浏览全部文件、点开任意文件看内容；
问"这个项目的启动流程是怎样的？"，agent 自己翻代码后给出正确回答。

## M3 — 能改代码 + Git 感知 ✅（v0.3，2026-06-11 验收通过）

价值跃迁点：从"问答工具"变成"coding agent"。Git 能力和 diff 审批天然一体，合并推进。

**Git 感知（先做，读写都依赖它）**
1. `git/` 模块：封装 git CLI（status / branch / diff / log，只读）
2. 顶栏显示当前分支；改动文件列表视图（git status，类似 IDE 的 Changes 面板）
3. 文件树角标：M（修改）/ A（新增）/ ?（未跟踪）
4. agent 只读工具：git_status / git_diff / git_log（agent 能感知"用户正在改什么"）

**写能力 + 审批**
5. 写工具：write_file / edit_file（精确字符串替换）
6. `security/policy.rs`：审批机制（PermissionAsk 事件 + oneshot 等待）
7. diff 计算（similar crate）+ 前端 diff 审查视图（approve / reject）
8. 会话级"全部允许"开关
9. 文件 watcher（notify crate）：agent/用户改动文件后，文件树、角标、改动列表自动刷新

**验收**：打开有改动的 git 项目，顶栏显示分支、树上有角标、能看改动列表；
让 agent "给 README 加一节使用说明"，弹出 diff，点确认后文件真的改了，角标随之更新。

## M4 — 能跑命令 ✅（v0.4，2026-06-11 实测自我纠错闭环）

1. `pty/manager.rs`：portable-pty 进程管理（spawn / 流式输出 / 超时 / kill）
2. bash 工具 + 命令审批
3. 前端 xterm.js 终端面板（回显执行过程）
4. agent 拿到命令输出后继续迭代（改错 → 重跑测试）

**验收**：让 agent "跑一下测试并修复失败的用例"，全程可见且可控。

## M5 — 能记住 ✅（v0.5，2026-06-11）

1. `session/store.rs`：SQLite 三表（sessions / messages / tool_calls）
2. 会话列表 UI：新建 / 切换 / 删除 / 改名
3. 重启 app 恢复历史会话
4. 上下文窗口管理：超长对话截断策略 + token 用量显示

**验收**：重启 app，昨天的会话还在，能接着聊。

## M6 — 能交付 目标：给别人用的 .dmg

1. 应用图标、窗口细节（标题栏、快捷键、深色模式）
2. 错误处理体检：断网 / key 失效 / 进程僵死等劣化路径
3. `npm run tauri build` 产出 .dmg，签名 + 公证（Apple Developer 账号）
4. GitHub Release + 简单落地页（README 完善）

---

## v2 里程碑（v1 交付后，顺序可再议）

- **M7 — 能上网**：web_fetch / web_search 工具（agent 查文档、搜报错）；
  之后可加内嵌浏览器面板预览 localhost
- **M8 — 能装技能**：skill 加载机制（SKILL.md 指令包，按需注入 prompt），
  对接 skillForge 技能库，两个项目打通
- **M9 — 能接生态**：MCP client（rmcp SDK），外部 MCP server 的工具
  动态注册进 tool registry

> v1 期间为此预留的接缝：tool registry 多来源设计（M2）、system prompt 分段组装（M2）。

## 里程碑之外的纪律

- 每个 M 结束打 git tag（v0.1 ~ v0.6），README 的功能清单同步更新
- Rust 是新语言：M1/M2 期间遇到所有权/生命周期问题，优先简单写法（clone 不丢人），先跑通再优化
- 每完成一个工具，写一个 Rust 单元测试（`cargo test` 保持绿）
