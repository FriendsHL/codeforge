import { Channel, invoke } from "@tauri-apps/api/core";

export interface ChatMessage {
  role: "user" | "assistant";
  content: string;
}

export type AgentEvent =
  | { type: "textDelta"; text: string }
  | { type: "reasoningDelta"; text: string }
  | { type: "turnEnd"; stopReason: string | null; outputTokens: number | null }
  | { type: "error"; message: string };

export async function sendMessage(
  provider: string,
  model: string,
  messages: ChatMessage[],
  onEvent: (event: AgentEvent) => void,
): Promise<void> {
  const channel = new Channel<AgentEvent>();
  channel.onmessage = onEvent;
  await invoke("send_message", { provider, model, messages, channel });
}

export const setApiKey = (key: string) => invoke<void>("set_api_key", { key });

export const hasApiKey = () => invoke<boolean>("has_api_key");
