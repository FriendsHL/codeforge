import { create } from "zustand";
import { gitFileDiff, readFilePreview } from "../lib/ipc";

/** 查看器内容：文件预览 / diff（浏览器与终端是独立的 tab，不走这里） */
export type ViewerContent =
  | { type: "file"; path: string; content: string; truncated: boolean }
  | { type: "diff"; path: string; diff: string };

/** 查看器栏的 tab 种类 */
export type ViewerTab = "content" | "browser" | "terminal";

interface ViewerState {
  /** 文件/diff 内容（null=该 tab 不存在） */
  content: ViewerContent | null;
  /** 浏览器地址（null=浏览器 tab 未开） */
  browserUrl: string | null;
  /** 终端 tab 是否开启 */
  terminalOpen: boolean;
  /** 当前激活的 tab */
  activeTab: ViewerTab;

  openFile: (path: string) => Promise<void>;
  openDiff: (path: string) => Promise<void>;
  openBrowser: (url?: string) => void;
  openTerminal: () => void;
  setTab: (tab: ViewerTab) => void;
  /** 关闭某个 tab；若关的是当前激活 tab，自动切到另一个还在的 tab */
  closeTab: (tab: ViewerTab) => void;
  /** 关闭整个查看器栏 */
  closeAll: () => void;
}

/** 关掉某 tab 后，挑一个还存在的 tab 作为新的激活 tab */
function fallbackTab(s: ViewerState, closing: ViewerTab): ViewerTab {
  const order: ViewerTab[] = ["content", "browser", "terminal"];
  const exists = (t: ViewerTab) =>
    t !== closing &&
    ((t === "content" && s.content !== null) ||
      (t === "browser" && s.browserUrl !== null) ||
      (t === "terminal" && s.terminalOpen));
  return order.find(exists) ?? "content";
}

/** 中栏与右栏之间的查看器：文件/diff、浏览器、终端三类 tab 同栏切换 */
export const useViewerStore = create<ViewerState>((set, get) => ({
  content: null,
  browserUrl: null,
  terminalOpen: false,
  activeTab: "content",

  openFile: async (path) => {
    const file = await readFilePreview(path);
    set({ content: { type: "file", path, ...file }, activeTab: "content" });
  },

  openDiff: async (path) => {
    const diff = await gitFileDiff(path);
    set({ content: { type: "diff", path, diff }, activeTab: "content" });
  },

  openBrowser: (url) =>
    set({
      browserUrl:
        url ?? localStorage.getItem("codeforge.browser.url") ?? "http://localhost:3000",
      activeTab: "browser",
    }),

  openTerminal: () => set({ terminalOpen: true, activeTab: "terminal" }),

  setTab: (tab) => set({ activeTab: tab }),

  closeTab: (tab) => {
    const s = get();
    const next = fallbackTab(s, tab);
    set({
      content: tab === "content" ? null : s.content,
      browserUrl: tab === "browser" ? null : s.browserUrl,
      terminalOpen: tab === "terminal" ? false : s.terminalOpen,
      activeTab: s.activeTab === tab ? next : s.activeTab,
    });
  },

  closeAll: () =>
    set({ content: null, browserUrl: null, terminalOpen: false, activeTab: "content" }),
}));
