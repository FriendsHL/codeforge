import { create } from "zustand";
import { GitChange, gitOverview } from "../lib/ipc";

interface GitState {
  branch: string | null;
  changes: GitChange[];
  refresh: () => Promise<void>;
}

export const useGitStore = create<GitState>((set) => ({
  branch: null,
  changes: [],

  refresh: async () => {
    try {
      const overview = await gitOverview();
      set({ branch: overview?.branch ?? null, changes: overview?.changes ?? [] });
    } catch {
      set({ branch: null, changes: [] });
    }
  },
}));
