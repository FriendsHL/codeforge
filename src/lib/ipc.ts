import { Channel, invoke } from "@tauri-apps/api/core";

export interface ChatMessage {
  role: "user" | "assistant";
  content: string;
}

export type AgentEvent =
  | { type: "textDelta"; text: string }
  | { type: "reasoningDelta"; text: string }
  | { type: "toolCallStart"; id: string; name: string; input: unknown }
  | { type: "toolCallEnd"; id: string; output: string; isError: boolean }
  | { type: "permissionAsk"; requestId: string; toolName: string; summary: string; diff: string }
  | { type: "commandOutput"; id: string; chunk: string }
  | { type: "turnEnd"; stopReason: string | null; outputTokens: number | null }
  | { type: "error"; message: string };

export interface WorkspaceInfo {
  root: string;
  name: string;
}

export interface TreeNode {
  path: string;
  name: string;
  isDir: boolean;
}

export interface FilePreview {
  content: string;
  truncated: boolean;
}

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

export const setWorkspace = (path: string) =>
  invoke<WorkspaceInfo>("set_workspace", { path });

export const readDirTree = (path: string) =>
  invoke<TreeNode[]>("read_dir_tree", { path });

export const readFilePreview = (path: string) =>
  invoke<FilePreview>("read_file_preview", { path });

export interface GitChange {
  path: string;
  status: string;
}

export interface GitOverview {
  branch: string;
  changes: GitChange[];
}

export const gitOverview = () => invoke<GitOverview | null>("git_overview");

export const gitFileDiff = (path: string) => invoke<string>("git_file_diff", { path });

export const approvePermission = (requestId: string, approved: boolean, allowAll: boolean) =>
  invoke<void>("approve_permission", { requestId, approved, allowAll });

export interface SessionMeta {
  id: number;
  title: string;
  updatedAt: string;
}

export const listSessions = () => invoke<SessionMeta[]>("list_sessions");

export const createSession = (title: string) =>
  invoke<SessionMeta>("create_session", { title });

export const renameSession = (id: number, title: string) =>
  invoke<void>("rename_session", { id, title });

export const deleteSession = (id: number) => invoke<void>("delete_session", { id });

export const loadSessionItems = (id: number) =>
  invoke<string>("load_session_items", { id });

export const saveSessionItems = (id: number, items: string) =>
  invoke<void>("save_session_items", { id, items });
