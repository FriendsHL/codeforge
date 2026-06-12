import { create } from "zustand";
import { gitFileDiff, readFilePreview } from "../lib/ipc";

export type ViewerContent =
  | { type: "file"; path: string; content: string; truncated: boolean }
  | { type: "diff"; path: string; diff: string };

interface ViewerState {
  content: ViewerContent | null;
  openFile: (path: string) => Promise<void>;
  openDiff: (path: string) => Promise<void>;
  close: () => void;
}

/** 中栏与右栏之间的内容查看器（文件预览 / diff），同一位置将来给浏览器面板复用 */
export const useViewerStore = create<ViewerState>((set) => ({
  content: null,

  openFile: async (path) => {
    const file = await readFilePreview(path);
    set({ content: { type: "file", path, ...file } });
  },

  openDiff: async (path) => {
    const diff = await gitFileDiff(path);
    set({ content: { type: "diff", path, diff } });
  },

  close: () => set({ content: null }),
}));
