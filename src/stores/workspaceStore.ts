import { create } from "zustand";
import { open } from "@tauri-apps/plugin-dialog";
import { setWorkspace } from "../lib/ipc";
import { useGitStore } from "./gitStore";

interface WorkspaceState {
  root: string | null;
  name: string | null;
  /** 每次打开/切换工作区递增，Explorer 据此重置树 */
  version: number;
  /** 弹文件夹选择器打开新项目 */
  openWorkspace: () => Promise<void>;
  /** 直接切到已知路径的项目（项目列表点击 / 会话联动） */
  switchTo: (path: string) => Promise<void>;
}

export const useWorkspaceStore = create<WorkspaceState>((set, get) => ({
  root: null,
  name: null,
  version: 0,

  openWorkspace: async () => {
    const selected = await open({ directory: true, multiple: false, title: "打开项目目录" });
    if (typeof selected !== "string") return;
    await get().switchTo(selected);
  },

  switchTo: async (path) => {
    if (path === get().root) return;
    const info = await setWorkspace(path);
    set({ root: info.root, name: info.name, version: get().version + 1 });
    void useGitStore.getState().refresh();
  },
}));
