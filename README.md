# ⚒️ CodeForge

A coding agent desktop app for macOS — sibling project of skillForge.

打开一个本地项目，和 agent 对话：它会自己读代码、改代码（经你审批）、跑测试自我纠错，全程可见可控。

## 功能（v1.1）

- **Agent loop**：自研 tool-use 循环，支持多轮工具链式调用（上限 30 轮）
- **内置工具**：read_file / list_dir / glob / grep / git_status / git_diff / git_log / write_file / edit_file / bash / web_fetch / web_search / todo_write，外加运行时注入的 browser_open / spawn_subagents 与 MCP 工具
- **联网**：web_search（Tavily 可选，DuckDuckGo 零配置兜底）+ web_fetch（HTML 转可读文本）
- **技能系统**：`<workspace>/.codeforge/skills/` 与 `~/.codeforge/skills/` 下的 SKILL.md 指令包，清单注入 prompt、正文按需加载；可直接放入 skillForge 技能
- **MCP client**：自研最小 stdio 实现（initialize / tools/list / tools/call），`mcp.json` 配置 server，工具动态进注册表（`mcp__server__tool`），统一走审批
- **多模型**：火山方舟 Ark（doubao / glm / kimi / deepseek / minimax）、小米 MiMo、Anthropic Claude，流式输出 + 推理过程展示
- **写操作审批**：改文件弹语法高亮 diff、跑命令弹完整命令，逐个允许或"本会话全部允许"
- **Git 感知**：顶栏分支、文件树 M/A/D/R/? 角标、改动列表点击看 diff、文件 watcher 自动刷新
- **IDE 式侧边栏**：会话 / 文件 / 改动 三页签；文件树懒加载、遵循 .gitignore、点击预览（语法高亮）
- **内嵌终端**：xterm.js 实时回显 agent 执行的命令（PTY，超时强制 kill）
- **会话持久化**：SQLite 存储，重启恢复，多会话切换/重命名/删除
- **安全**：API key 存 macOS Keychain；工具访问限定在工作区内（拒绝路径越界）

## 技术栈

- **Shell**: [Tauri 2](https://tauri.app)（Rust 后端，~10MB 包体）
- **前端**: React 18 + TypeScript + Vite + antd
- **Rust 侧**: tokio / reqwest(SSE) / portable-pty / rusqlite / notify / similar / keyring

## 配置

模型 API key（任选其一即可使用）：

| Provider | 配置方式 |
|---|---|
| 火山方舟 Ark | 环境变量 `ARK_API_KEY`（从配置了该变量的终端启动 app） |
| 小米 MiMo | 环境变量 `XIAOMI_MIMO_API_KEY` |
| Anthropic | app 内设置页填入，存 macOS Keychain |

## 开发

前置：Node.js ≥ 20、Rust 工具链（`rustup`）、Xcode Command Line Tools。

```bash
npm install
npm run tauri dev    # 开发模式（热更新）
npm run tauri build  # 产出 .app / .dmg（src-tauri/target/release/bundle/）
cd src-tauri && cargo test            # Rust 单元测试
cargo test -- --ignored --nocapture   # 真实 API 集成测试（需 ARK_API_KEY）
```

## 安装（未签名版本说明）

当前 .dmg 未做 Apple 签名/公证，首次打开需：右键 CodeForge.app → 打开 → 再点"打开"。

## 项目结构

```
src/                 # React 前端（components/stores/lib）
src-tauri/src/
  agent/             # 核心循环、事件协议、system prompt
  llm/               # Anthropic + OpenAI 兼容流式客户端、provider 注册表
  tools/             # Tool trait + 10 个内置工具（多来源注册表，预留 MCP/skill）
  security/          # 写操作审批（oneshot + 会话级 allow-all）
  pty/               # portable-pty 命令执行
  git/               # git CLI 封装
  session/           # SQLite 会话持久化
  commands/          # Tauri IPC 入口层
docs/                # ARCHITECTURE.md / ROADMAP.md
```

## MCP 配置示例

设置页可查看配置文件路径（`~/Library/Application Support/com.codeforge.desktop/mcp.json`）：

```json
{
  "servers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
    }
  }
}
```

编辑后在设置里点"重新加载"。

## Roadmap

v1（M1–M6）与 v2（M7 web / M8 skills / M9 MCP）均已完成，详见 [docs/ROADMAP.md](docs/ROADMAP.md)。
