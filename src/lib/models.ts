// 模型清单与纯函数（无副作用，便于单测）。value 格式 "<provider>/<model>"，
// provider 对应 Rust 端 llm/registry.rs。

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

export const DEFAULT_MODEL = "ark/doubao-seed-2.0-pro";
export const MODEL_STORAGE_KEY = "codeforge.model";

export function splitModelValue(value: string): { provider: string; model: string } {
  const slash = value.indexOf("/");
  return { provider: value.slice(0, slash), model: value.slice(slash + 1) };
}

// 各模型大致的上下文窗口（tokens），用于在界面上把"当前上下文占用"换算成百分比。
// 取保守值，不必精确——只为给用户一个"还剩多少空间"的直觉。
const CONTEXT_WINDOWS: Record<string, number> = {
  "claude-opus-4-8": 1_000_000,
  "claude-sonnet-4-6": 1_000_000,
  "claude-haiku-4-5": 200_000,
  "minimax-latest": 1_000_000,
  "deepseek-v4-pro": 128_000,
  "glm-5.1": 200_000,
};

const DEFAULT_CONTEXT_WINDOW = 256_000;

/** 给定 "<provider>/<model>" 或裸 model 名，返回其上下文窗口大小（tokens） */
export function contextWindowFor(value: string): number {
  const model = value.includes("/") ? splitModelValue(value).model : value;
  return CONTEXT_WINDOWS[model] ?? DEFAULT_CONTEXT_WINDOW;
}
