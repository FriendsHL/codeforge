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

/** UI 层消息：在 API 消息之上多一段推理过程展示 */
export interface UiMessage extends ChatMessage {
  reasoning?: string;
}

interface ChatState {
  messages: UiMessage[];
  streaming: boolean;
  error: string | null;
  model: string;
  setModel: (model: string) => void;
  send: (text: string) => Promise<void>;
  clear: () => void;
}

export const useChatStore = create<ChatState>((set, get) => ({
  messages: [],
  streaming: false,
  error: null,
  model: localStorage.getItem(MODEL_STORAGE_KEY) ?? DEFAULT_MODEL,

  setModel: (model) => {
    localStorage.setItem(MODEL_STORAGE_KEY, model);
    set({ model });
  },

  send: async (text) => {
    const { messages, model, streaming } = get();
    if (streaming || !text.trim()) return;

    const history: ChatMessage[] = [
      ...messages.map(({ role, content }) => ({ role, content })),
      { role: "user" as const, content: text },
    ];
    set({
      messages: [
        ...messages,
        { role: "user", content: text },
        { role: "assistant", content: "" },
      ],
      streaming: true,
      error: null,
    });

    const patchLast = (patch: (last: UiMessage) => UiMessage) =>
      set((state) => {
        const next = [...state.messages];
        next[next.length - 1] = patch(next[next.length - 1]);
        return { messages: next };
      });

    try {
      const { provider, model: modelId } = splitModelValue(model);
      await sendMessage(provider, modelId, history, (event) => {
        if (event.type === "textDelta") {
          patchLast((m) => ({ ...m, content: m.content + event.text }));
        } else if (event.type === "reasoningDelta") {
          patchLast((m) => ({ ...m, reasoning: (m.reasoning ?? "") + event.text }));
        } else if (event.type === "error") {
          set({ error: event.message });
        }
      });
    } catch (e) {
      set({ error: String(e) });
    } finally {
      set((state) => {
        const next = [...state.messages];
        const last = next[next.length - 1];
        // 失败且没流出任何内容时，去掉空的 assistant 占位气泡
        if (last?.role === "assistant" && !last.content && !last.reasoning) {
          next.pop();
        }
        return { messages: next, streaming: false };
      });
    }
  },

  clear: () => set({ messages: [], error: null }),
}));
