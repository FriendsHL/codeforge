import { create } from "zustand";
import {
  deleteSession,
  listProjects,
  listSessions,
  loadSessionItems,
  ProjectMeta,
  removeProject,
  renameSession,
  SessionMeta,
} from "../lib/ipc";
import { useChatStore } from "./chatStore";
import { useWorkspaceStore } from "./workspaceStore";

interface SessionState {
  sessions: SessionMeta[];
  projects: ProjectMeta[];
  refresh: () => Promise<void>;
  /** 切换到历史会话；会话属于其他项目时自动切工作区 */
  open: (id: number) => Promise<void>;
  /** 开新会话（清空当前区，首条消息发出时才落库，归属当前项目） */
  startNew: () => void;
  rename: (id: number, title: string) => Promise<void>;
  remove: (id: number) => Promise<void>;
  /** 从最近列表移除项目（不删磁盘文件、不删其会话） */
  forgetProject: (root: string) => Promise<void>;
}

export const useSessionStore = create<SessionState>((set, get) => ({
  sessions: [],
  projects: [],

  refresh: async () => {
    const [sessions, projects] = await Promise.all([listSessions(), listProjects()]);
    set({ sessions, projects });
  },

  open: async (id) => {
    const meta = get().sessions.find((s) => s.id === id);
    // 会话归属其他项目 → 联动切换工作区
    if (meta?.workspaceRoot && meta.workspaceRoot !== useWorkspaceStore.getState().root) {
      await useWorkspaceStore.getState().switchTo(meta.workspaceRoot);
    }
    const json = await loadSessionItems(id);
    useChatStore.setState({
      items: JSON.parse(json),
      currentSessionId: id,
      error: null,
      sessionTokens: 0,
      sessionInputTokens: 0,
      turnInputTokens: 0,
      turnOutputTokens: 0,
      contextTokens: null,
      terminalOpen: false,
    });
  },

  startNew: () => {
    useChatStore.setState({
      items: [],
      currentSessionId: null,
      error: null,
      sessionTokens: 0,
      sessionInputTokens: 0,
      turnInputTokens: 0,
      turnOutputTokens: 0,
      contextTokens: null,
      terminalOpen: false,
    });
  },

  rename: async (id, title) => {
    await renameSession(id, title);
    await get().refresh();
  },

  remove: async (id) => {
    await deleteSession(id);
    if (useChatStore.getState().currentSessionId === id) {
      get().startNew();
    }
    await get().refresh();
  },

  forgetProject: async (root) => {
    await removeProject(root);
    await get().refresh();
  },
}));
