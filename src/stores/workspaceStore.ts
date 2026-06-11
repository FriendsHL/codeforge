import { create } from "zustand";
import { open } from "@tauri-apps/plugin-dialog";
import { setWorkspace } from "../lib/ipc";

interface WorkspaceState {
  root: string | null;
  name: string | null;
  /** 每次打开/切换工作区递增，Explorer 据此重置树 */
  version: number;
  openWorkspace: () => Promise<void>;
}

export const useWorkspaceStore = create<WorkspaceState>((set, get) => ({
  root: null,
  name: null,
  version: 0,

  openWorkspace: async () => {
    const selected = await open({ directory: true, multiple: false, title: "打开项目目录" });
    if (typeof selected !== "string") return;
    const info = await setWorkspace(selected);
    set({ root: info.root, name: info.name, version: get().version + 1 });
  },
}));
