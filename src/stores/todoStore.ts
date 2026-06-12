import { create } from "zustand";

export interface TodoItem {
  content: string;
  status: "pending" | "in_progress" | "completed";
}

interface TodoState {
  todos: TodoItem[];
  set: (todos: TodoItem[]) => void;
  clear: () => void;
}

/** agent 的任务清单（todo_write 工具推送，每个回合开始时清空） */
export const useTodoStore = create<TodoState>((set) => ({
  todos: [],
  set: (todos) => set({ todos }),
  clear: () => set({ todos: [] }),
}));
