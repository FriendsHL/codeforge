import { create } from "zustand";

export interface TeamTask {
  id: string;
  title: string;
  role: string | null;
  status: "running" | "done" | "failed" | "cancelled";
  result: string | null;
}

interface TeamState {
  tasks: TeamTask[];
  set: (tasks: TeamTask[]) => void;
  clear: () => void;
}

/** 异步团队任务看板（spawn_team/team_status 推送 TeamUpdate 事件刷新） */
export const useTeamStore = create<TeamState>((set) => ({
  tasks: [],
  set: (tasks) => set({ tasks }),
  clear: () => set({ tasks: [] }),
}));
