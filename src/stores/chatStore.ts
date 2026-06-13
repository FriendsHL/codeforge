import { create } from "zustand";
import {
  ChatMessage,
  createSession,
  listCapabilities,
  saveSessionItems,
  sendMessage,
} from "../lib/ipc";
import { invoke } from "@tauri-apps/api/core";
import { termWrite } from "../lib/terminal";
import { useGitStore } from "./gitStore";
import { useSessionStore } from "./sessionStore";
import { useTodoStore } from "./todoStore";

const SLASH_HELP = `可用快捷命令：
- \`/tools\` 当前可用工具清单
- \`/skills\` 当前已装技能
- \`/mcp\` MCP server 连接状态
- \`/model [名称]\` 查看/切换模型
- \`/compact\` 手动压缩当前会话
- \`/trace\` 本会话的 token/工具耗时统计
- \`/clear\` 开一个新会话
- \`/help\` 本帮助`;

/** 本地快捷命令（不经过 LLM）。返回 null 表示不是已知命令 */
async function runSlashCommand(text: string): Promise<string | null> {
  const cmd = text.split(/\s/)[0].toLowerCase();
  switch (cmd) {
    case "/help":
      return SLASH_HELP;
    case "/tools": {
      const caps = await listCapabilities();
      const rows = caps.tools.map((t) => `| \`${t.name}\` | ${t.description} |`).join("\n");
      return `**当前可用工具（${caps.tools.length} 个）**\n\n| 工具 | 说明 |\n|---|---|\n${rows}`;
    }
    case "/skills": {
      const caps = await listCapabilities();
      if (caps.skills.length === 0) {
        return "当前没有已安装的技能。把 SKILL.md 技能包放进 `~/.codeforge/skills/` 或项目的 `.codeforge/skills/`。";
      }
      const rows = caps.skills.map((s) => `| \`${s.name}\` | ${s.description} |`).join("\n");
      return `**当前技能（${caps.skills.length} 个）**\n\n| 技能 | 说明 |\n|---|---|\n${rows}`;
    }
    case "/mcp": {
      const servers = await invoke<{ name: string; connected: boolean; toolCount: number; error: string | null }[]>("mcp_status");
      if (servers.length === 0) {
        return "未配置 MCP server。在设置里查看 mcp.json 路径，配置后点「重新加载」。";
      }
      return servers
        .map((s) =>
          s.connected
            ? `- ✅ **${s.name}**：${s.toolCount} 个工具`
            : `- ❌ **${s.name}**：${s.error ?? "连接失败"}`,
        )
        .join("\n");
    }
    default:
      return cmd.startsWith("/") ? `未知命令 \`${cmd}\`\n\n${SLASH_HELP}` : null;
  }
}

interface SpanRow {
  name: string;
  durationMs: number;
  isError: boolean;
}
interface TraceSummaryData {
  turns: number;
  llmCalls: number;
  inputTokens: number;
  outputTokens: number;
  toolCalls: number;
  totalMs: number;
  slowestTools: SpanRow[];
  waterfall: SpanRow[];
}

function fmtMs(ms: number): string {
  return ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${ms}ms`;
}

/** /trace：把聚合数据渲染成 markdown（含 ASCII 瀑布条） */
async function formatTrace(sessionId: number): Promise<string> {
  const t = await invoke<TraceSummaryData>("trace_summary", { sessionId });
  if (t.turns === 0 && t.waterfall.length === 0) {
    return "本会话还没有 trace 记录（发一条消息后再看）。";
  }
  const overview =
    `**本会话 trace**\n\n` +
    `| 指标 | 值 |\n|---|---|\n` +
    `| 回合数 | ${t.turns} |\n` +
    `| LLM 调用 | ${t.llmCalls} |\n` +
    `| 输入 tokens | ${t.inputTokens.toLocaleString()} |\n` +
    `| 输出 tokens | ${t.outputTokens.toLocaleString()} |\n` +
    `| 工具调用 | ${t.toolCalls} |\n` +
    `| 累计耗时 | ${fmtMs(t.totalMs)} |`;

  let slowest = "";
  if (t.slowestTools.length > 0) {
    const maxMs = Math.max(...t.slowestTools.map((s) => s.durationMs), 1);
    const lines = t.slowestTools
      .map((s) => {
        const bars = "█".repeat(Math.max(1, Math.round((s.durationMs / maxMs) * 18)));
        return `${fmtMs(s.durationMs).padStart(7)}  ${bars} ${s.name}${s.isError ? " (err)" : ""}`;
      })
      .join("\n");
    slowest = `\n\n**最慢工具**\n\n\`\`\`\n${lines}\n\`\`\``;
  }
  return overview + slowest;
}

// value 格式: "<provider>/<model>"，provider 对应 Rust 端 llm/registry.rs
export const MODEL_GROUPS = [
  {
    label: "火山方舟 Ark（ARK_API_KEY）",
    options: [
      { value: "ark/doubao-seed-2.0-pro", label: "Doubao Seed 2.0 Pro" },
      { value: "ark/doubao-seed-2.0-code", label: "Doubao Seed 2.0 Code" },
      { value: "ark/doubao-seed-2.0-lite", label: "Doubao Seed 2.0 Lite" },
      { value: "ark/glm-5.1", label: "GLM 5.1" },
      { value: "ark/kimi-k2.6", label: "Kimi K2.6" },
      { value: "ark/deepseek-v4-pro", label: "DeepSeek V4 Pro" },
      { value: "ark/minimax-latest", label: "MiniMax (latest)" },
    ],
  },
  {
    label: "小米 MiMo（XIAOMI_MIMO_API_KEY）",
    options: [
      { value: "xiaomi-mimo/mimo-v2.5-pro", label: "MiMo V2.5 Pro" },
      { value: "xiaomi-mimo/mimo-v2.5", label: "MiMo V2.5" },
    ],
  },
  {
    label: "Anthropic（Keychain）",
    options: [
      { value: "claude/claude-opus-4-8", label: "Claude Opus 4.8" },
      { value: "claude/claude-sonnet-4-6", label: "Claude Sonnet 4.6" },
      { value: "claude/claude-haiku-4-5", label: "Claude Haiku 4.5" },
    ],
  },
];

const DEFAULT_MODEL = "ark/doubao-seed-2.0-pro";
const MODEL_STORAGE_KEY = "codeforge.model";

export function splitModelValue(value: string): { provider: string; model: string } {
  const slash = value.indexOf("/");
  return { provider: value.slice(0, slash), model: value.slice(slash + 1) };
}

export type ChatItem =
  | {
      kind: "msg";
      role: "user" | "assistant";
      content: string;
      reasoning?: string;
      mentions?: string[];
      /** 生成中追加、尚未被 agent 处理的消息 */
      queued?: boolean;
    }
  | { kind: "notice"; text: string }
  | {
      kind: "tool";
      id: string;
      name: string;
      input: unknown;
      output?: string;
      isError?: boolean;
      durationMs?: number;
      checkpointId?: string | null;
      reverted?: boolean;
      done: boolean;
    }
  | {
      kind: "approval";
      requestId: string;
      toolName: string;
      summary: string;
      diff: string;
      danger?: string | null;
      decision?: "approved" | "denied" | "allowAll";
    };

interface ChatState {
  items: ChatItem[];
  streaming: boolean;
  error: string | null;
  model: string;
  terminalOpen: boolean;
  currentSessionId: number | null;
  /** 本会话累计输出 tokens（仅 UI 提示用） */
  sessionTokens: number;
  /** 最近一次请求的真实上下文大小（API usage.input_tokens） */
  contextTokens: number | null;
  setModel: (model: string) => void;
  /** mentions: @ 引用的文件相对路径，内容会注入本轮上下文 */
  send: (text: string, mentions?: string[]) => Promise<void>;
  /** 生成中追加消息：排队注入到后续轮次。返回 false 表示当前没有活动回合 */
  queue: (text: string) => Promise<boolean>;
  clear: () => void;
}

export const useChatStore = create<ChatState>((set, get) => ({
  items: [],
  streaming: false,
  error: null,
  model: localStorage.getItem(MODEL_STORAGE_KEY) ?? DEFAULT_MODEL,
  terminalOpen: false,
  currentSessionId: null,
  sessionTokens: 0,
  contextTokens: null,

  setModel: (model) => {
    localStorage.setItem(MODEL_STORAGE_KEY, model);
    set({ model });
  },

  send: async (text, mentions = []) => {
    const { items, model, streaming } = get();
    if (streaming || !text.trim()) return;

    // 快捷命令本地处理，不经过 LLM、不落库
    const trimmed = text.trim();
    if (trimmed.startsWith("/")) {
      const cmd = trimmed.split(/\s/)[0].toLowerCase();
      const arg = trimmed.slice(cmd.length).trim();
      const echo = (reply: string) =>
        set((s) => ({
          items: [
            ...s.items,
            { kind: "msg", role: "user", content: trimmed },
            { kind: "msg", role: "assistant", content: reply },
          ],
        }));

      // 需要 store 状态的命令
      if (cmd === "/clear") {
        useSessionStore.getState().startNew();
        return;
      }
      if (cmd === "/model") {
        if (!arg) {
          const all = MODEL_GROUPS.flatMap((g) => g.options);
          const list = all
            .map((o) => `- ${o.value === model ? "**▸ " : ""}\`${o.value}\`${o.value === model ? "**（当前）" : ""} — ${o.label}`)
            .join("\n");
          echo(`**当前模型** \`${model}\`\n\n切换：\`/model <名称片段>\`\n\n${list}`);
          return;
        }
        const all = MODEL_GROUPS.flatMap((g) => g.options);
        const hit =
          all.find((o) => o.value === arg) ??
          all.find((o) => o.value.toLowerCase().includes(arg.toLowerCase()) || o.label.toLowerCase().includes(arg.toLowerCase()));
        if (hit) {
          get().setModel(hit.value);
          echo(`已切换模型为 \`${hit.value}\`（${hit.label}）`);
        } else {
          echo(`没找到匹配「${arg}」的模型，用 \`/model\` 看全部。`);
        }
        return;
      }
      if (cmd === "/compact") {
        const msgs = get()
          .items.filter((i): i is Extract<ChatItem, { kind: "msg" }> => i.kind === "msg")
          .filter((i) => i.content.trim() !== "");
        if (msgs.length < 2) {
          echo("当前会话太短，无需压缩。");
          return;
        }
        set({ items: [...get().items, { kind: "msg", role: "user", content: trimmed }] });
        try {
          const { provider, model: modelId } = splitModelValue(model);
          void modelId;
          const summary = await invoke<string>("compact_now", {
            provider,
            messages: msgs.map(({ role, content }) => ({ role, content })),
          });
          set({
            items: [
              { kind: "notice", text: "🗜 已手动压缩：以下为整段会话的结构化摘要，后续对话基于它继续" },
              { kind: "msg", role: "assistant", content: summary },
            ],
          });
          const sid = get().currentSessionId;
          if (sid !== null) {
            await saveSessionItems(sid, JSON.stringify(get().items));
            void useSessionStore.getState().refresh();
          }
        } catch (e) {
          set({ error: `压缩失败: ${e}` });
        }
        return;
      }
      if (cmd === "/trace") {
        const sid = get().currentSessionId;
        if (sid === null) {
          echo("当前还没有会话记录。");
          return;
        }
        try {
          echo(await formatTrace(sid));
        } catch (e) {
          echo(`读取 trace 失败: ${e}`);
        }
        return;
      }

      // 无状态命令
      const reply = await runSlashCommand(trimmed);
      if (reply !== null) {
        echo(reply);
        return;
      }
    }

    // 首条消息时落库建会话（标题取消息前 24 字，归属当前项目）
    if (get().currentSessionId === null) {
      try {
        const { useWorkspaceStore } = await import("./workspaceStore");
        const meta = await createSession(
          text.trim().slice(0, 24),
          useWorkspaceStore.getState().root,
        );
        set({ currentSessionId: meta.id });
        void useSessionStore.getState().refresh();
      } catch (e) {
        set({ error: `创建会话失败: ${e}` });
        return;
      }
    }

    // @ 引用的文件内容注入本轮（仅本轮，不污染展示文本；后续轮 agent 可自行 read_file）
    let injected = text;
    if (mentions.length > 0) {
      try {
        const { readFilesForContext } = await import("../lib/ipc");
        const block = await readFilesForContext(mentions);
        injected = `${text}\n\n[用户引用的文件内容]\n${block}`;
      } catch (e) {
        set({ error: `读取引用文件失败: ${e}` });
        return;
      }
    }

    // 跨轮次历史只保留纯文本消息（工具轮次每次由 Rust 端 loop 内部重建；notice 不进历史）
    const history: ChatMessage[] = [
      ...get()
        .items.filter((i): i is Extract<ChatItem, { kind: "msg" }> => i.kind === "msg")
        .filter((i) => i.content.trim() !== "")
        .map(({ role, content }) => ({ role, content })),
      { role: "user" as const, content: injected },
    ];

    useTodoStore.getState().clear(); // 新回合清空旧任务清单
    set({
      // 展示用原文（带 @path），气泡下方另列引用的文件
      items: [...items, { kind: "msg", role: "user", content: text, mentions }],
      streaming: true,
      error: null,
    });

    const update = (updater: (items: ChatItem[]) => ChatItem[]) =>
      set((state) => ({ items: updater([...state.items]) }));

    // textDelta/reasoningDelta 追加到末尾的 assistant 气泡；
    // 若末尾不是 assistant（如刚执行完工具），就新开一个气泡
    const appendToAssistant = (patch: Partial<Extract<ChatItem, { kind: "msg" }>>) =>
      update((items) => {
        const last = items[items.length - 1];
        if (last?.kind === "msg" && last.role === "assistant") {
          items[items.length - 1] = {
            ...last,
            content: last.content + (patch.content ?? ""),
            reasoning: (last.reasoning ?? "") + (patch.reasoning ?? "") || last.reasoning,
          };
        } else {
          items.push({
            kind: "msg",
            role: "assistant",
            content: patch.content ?? "",
            reasoning: patch.reasoning,
          });
        }
        return items;
      });

    try {
      const { provider, model: modelId } = splitModelValue(model);
      await sendMessage(provider, modelId, history, get().currentSessionId, (event) => {
        switch (event.type) {
          case "textDelta":
            appendToAssistant({ content: event.text });
            break;
          case "reasoningDelta":
            appendToAssistant({ reasoning: event.text });
            break;
          case "toolCallStart":
            update((items) => {
              items.push({
                kind: "tool",
                id: event.id,
                name: event.name,
                input: event.input,
                done: false,
              });
              return items;
            });
            if (event.name === "bash") {
              const cmd = (event.input as { command?: string })?.command ?? "";
              termWrite(`\r\n\x1b[1;33m$ ${cmd}\x1b[0m\r\n`);
              set({ terminalOpen: true });
            }
            break;
          case "commandOutput":
            termWrite(event.chunk);
            break;
          case "toolCallEnd":
            update((items) =>
              items.map((item) =>
                item.kind === "tool" && item.id === event.id
                  ? {
                      ...item,
                      output: event.output,
                      isError: event.isError,
                      durationMs: event.durationMs,
                      checkpointId: event.checkpointId,
                      done: true,
                    }
                  : item,
              ),
            );
            break;
          case "permissionAsk":
            update((items) => {
              items.push({
                kind: "approval",
                requestId: event.requestId,
                toolName: event.toolName,
                summary: event.summary,
                diff: event.diff,
                danger: event.danger,
              });
              return items;
            });
            break;
          case "contextCompacted":
            update((items) => {
              items.push({ kind: "notice", text: `🗜 ${event.note}` });
              return items;
            });
            break;
          case "turnEnd":
            if (event.outputTokens) {
              set((s) => ({ sessionTokens: s.sessionTokens + (event.outputTokens ?? 0) }));
            }
            if (event.inputTokens) {
              set({ contextTokens: event.inputTokens });
            }
            // 把"为什么停"显式标注出来，不再让用户猜
            if (event.stopReason === "length" || event.stopReason === "max_tokens") {
              appendToAssistant({
                content: "\n\n> ⚠️ 输出达到单轮 max_tokens 上限被截断，可以说「继续」让我接着写",
              });
            } else if (event.stopReason === "cancelled") {
              appendToAssistant({ content: "\n\n> ⏹ 已手动停止" });
            }
            break;
          case "error":
            set({ error: event.message });
            break;
        }
      });
    } catch (e) {
      set({ error: String(e) });
    } finally {
      set((state) => {
        const next = [...state.items];
        const last = next[next.length - 1];
        if (last?.kind === "msg" && last.role === "assistant" && !last.content && !last.reasoning) {
          next.pop();
        }
        return { items: next, streaming: false };
      });
      // 每轮结束刷新 git 状态（用户可能在外部改了文件；M3 写能力上线后 agent 也会改）
      void useGitStore.getState().refresh();
      // 持久化本轮完整对话
      const { currentSessionId, items: finalItems } = get();
      if (currentSessionId !== null) {
        try {
          await saveSessionItems(currentSessionId, JSON.stringify(finalItems));
          void useSessionStore.getState().refresh();
        } catch {
          // 持久化失败不打断对话
        }
      }
    }
  },

  queue: async (text) => {
    const t = text.trim();
    if (!t) return false;
    const { queueUserMessage } = await import("../lib/ipc");
    const ok = await queueUserMessage(t);
    if (ok) {
      // 乐观插入用户气泡，标记为排队中
      set((s) => ({
        items: [...s.items, { kind: "msg", role: "user", content: t, queued: true }],
      }));
    }
    return ok;
  },

  clear: () => useSessionStore.getState().startNew(),
}));
