import { create } from "zustand";
import { ChatMessage, sendMessage } from "../lib/ipc";

export const MODELS = [
  { value: "claude-opus-4-8", label: "Claude Opus 4.8（最强）" },
  { value: "claude-sonnet-4-6", label: "Claude Sonnet 4.6（均衡）" },
  { value: "claude-haiku-4-5", label: "Claude Haiku 4.5（最快）" },
];

const MODEL_STORAGE_KEY = "codeforge.model";

interface ChatState {
  messages: ChatMessage[];
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
  model: localStorage.getItem(MODEL_STORAGE_KEY) ?? MODELS[0].value,

  setModel: (model) => {
    localStorage.setItem(MODEL_STORAGE_KEY, model);
    set({ model });
  },

  send: async (text) => {
    const { messages, model, streaming } = get();
    if (streaming || !text.trim()) return;

    const history: ChatMessage[] = [...messages, { role: "user", content: text }];
    set({
      messages: [...history, { role: "assistant", content: "" }],
      streaming: true,
      error: null,
    });

    const appendDelta = (delta: string) =>
      set((state) => {
        const next = [...state.messages];
        const last = next[next.length - 1];
        next[next.length - 1] = { ...last, content: last.content + delta };
        return { messages: next };
      });

    try {
      await sendMessage(model, history, (event) => {
        if (event.type === "textDelta") appendDelta(event.text);
        else if (event.type === "error") set({ error: event.message });
      });
    } catch (e) {
      set({ error: String(e) });
    } finally {
      set((state) => {
        const next = [...state.messages];
        // 失败且没流出任何内容时，去掉空的 assistant 占位气泡
        if (next[next.length - 1]?.role === "assistant" && !next[next.length - 1].content) {
          next.pop();
        }
        return { messages: next, streaming: false };
      });
    }
  },

  clear: () => set({ messages: [], error: null }),
}));
