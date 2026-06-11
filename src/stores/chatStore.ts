import { create } from "zustand";
import { ChatMessage, sendMessage } from "../lib/ipc";

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
  | { kind: "msg"; role: "user" | "assistant"; content: string; reasoning?: string }
  | {
      kind: "tool";
      id: string;
      name: string;
      input: unknown;
      output?: string;
      isError?: boolean;
      done: boolean;
    };

interface ChatState {
  items: ChatItem[];
  streaming: boolean;
  error: string | null;
  model: string;
  setModel: (model: string) => void;
  send: (text: string) => Promise<void>;
  clear: () => void;
}

export const useChatStore = create<ChatState>((set, get) => ({
  items: [],
  streaming: false,
  error: null,
  model: localStorage.getItem(MODEL_STORAGE_KEY) ?? DEFAULT_MODEL,

  setModel: (model) => {
    localStorage.setItem(MODEL_STORAGE_KEY, model);
    set({ model });
  },

  send: async (text) => {
    const { items, model, streaming } = get();
    if (streaming || !text.trim()) return;

    // 跨轮次历史只保留纯文本消息（工具轮次每次由 Rust 端 loop 内部重建）
    const history: ChatMessage[] = [
      ...items
        .filter((i): i is Extract<ChatItem, { kind: "msg" }> => i.kind === "msg")
        .filter((i) => i.content.trim() !== "")
        .map(({ role, content }) => ({ role, content })),
      { role: "user" as const, content: text },
    ];

    set({
      items: [...items, { kind: "msg", role: "user", content: text }],
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
      await sendMessage(provider, modelId, history, (event) => {
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
            break;
          case "toolCallEnd":
            update((items) =>
              items.map((item) =>
                item.kind === "tool" && item.id === event.id
                  ? { ...item, output: event.output, isError: event.isError, done: true }
                  : item,
              ),
            );
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
    }
  },

  clear: () => set({ items: [], error: null }),
}));
