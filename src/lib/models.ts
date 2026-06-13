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
