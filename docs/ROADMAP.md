# codeForge 开发路线图

原则：**每个里程碑结束时 app 都能跑、都有肉眼可见的新能力**（纵向切片，不做横向铺设）。
M1 是最薄的端到端打通，后面每步在其上叠加。

## M1 — 能聊天 ✅ 目标：窗口里和 Claude 流式对话

最小闭环，打通 前端 → IPC → Rust → Anthropic API → 流式事件 → UI 全链路。

1. 前端布局骨架：会话区 + 输入框 + 设置页（antd）
2. `config/`：API key 存取（keyring → macOS Keychain）+ 设置页对接
3. `llm/anthropic.rs`：Messages API 流式客户端（reqwest + SSE）
4. `commands/chat.rs`：send_message + Channel 推 TextDelta 事件
5. 前端流式渲染（react-markdown，打字机效果）

**验收**：填入 API key，发"你好"，看到流式回复。

## M2 — 能看代码 目标：agent 自主读代码并回答项目问题

引入 agent loop 和只读工具，这是项目的心脏。

1. `tools/registry.rs`：Tool trait + JSON Schema 注册表
2. 只读工具：read_file / list_dir / glob / grep
3. `agent/loop_.rs`：完整 tool-use 循环（LLM 返回 tool_use → 执行 → 回填 → 再调 LLM）
4. 工作区：打开项目目录（文件夹选择器），工具限定在目录内
5. UI：工具调用卡片（显示 agent 正在读哪个文件、搜什么）

**验收**：打开 skillForge 目录，问"这个项目的启动流程是怎样的？"，agent 自己翻代码后给出正确回答。

## M3 — 能改代码 目标：agent 写代码，用户审批 diff

价值跃迁点：从"问答工具"变成"coding agent"。

1. 写工具：write_file / edit_file（精确字符串替换）
2. `security/policy.rs`：审批机制（PermissionAsk 事件 + oneshot 等待）
3. diff 计算（similar crate）+ 前端 diff 审查视图（approve / reject）
4. 会话级"全部允许"开关

**验收**：让 agent "给 README 加一节使用说明"，弹出 diff，点确认后文件真的改了。

## M4 — 能跑命令 目标：agent 执行测试/构建，形成自我纠错闭环

1. `pty/manager.rs`：portable-pty 进程管理（spawn / 流式输出 / 超时 / kill）
2. bash 工具 + 命令审批
3. 前端 xterm.js 终端面板（回显执行过程）
4. agent 拿到命令输出后继续迭代（改错 → 重跑测试）

**验收**：让 agent "跑一下测试并修复失败的用例"，全程可见且可控。

## M5 — 能记住 目标：多会话 + 历史持久化

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

## 里程碑之外的纪律

- 每个 M 结束打 git tag（v0.1 ~ v0.6），README 的功能清单同步更新
- Rust 是新语言：M1/M2 期间遇到所有权/生命周期问题，优先简单写法（clone 不丢人），先跑通再优化
- 每完成一个工具，写一个 Rust 单元测试（`cargo test` 保持绿）
