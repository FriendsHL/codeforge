import { create } from "zustand";
import {
  deleteSession,
  listSessions,
  loadSessionItems,
  renameSession,
  SessionMeta,
} from "../lib/ipc";
import { useChatStore } from "./chatStore";

interface SessionState {
  sessions: SessionMeta[];
  refresh: () => Promise<void>;
  /** 切换到历史会话（加载其消息） */
  open: (id: number) => Promise<void>;
  /** 开新会话（清空当前区，首条消息发出时才落库） */
  startNew: () => void;
  rename: (id: number, title: string) => Promise<void>;
  remove: (id: number) => Promise<void>;
}

export const useSessionStore = create<SessionState>((set, get) => ({
  sessions: [],

  refresh: async () => {
    set({ sessions: await listSessions() });
  },

  open: async (id) => {
    const json = await loadSessionItems(id);
    useChatStore.setState({
      items: JSON.parse(json),
      currentSessionId: id,
      error: null,
      sessionTokens: 0,
      terminalOpen: false,
    });
  },

  startNew: () => {
    useChatStore.setState({
      items: [],
      currentSessionId: null,
      error: null,
      sessionTokens: 0,
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
}));
