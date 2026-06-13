import { create } from "zustand";

export type ThemeMode = "light" | "dark";

const KEY = "codeforge.theme";

function initial(): ThemeMode {
  const saved = localStorage.getItem(KEY);
  if (saved === "dark" || saved === "light") return saved;
  // 跟随系统
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

interface ThemeState {
  mode: ThemeMode;
  toggle: () => void;
}

export const useThemeStore = create<ThemeState>((set, get) => ({
  mode: initial(),
  toggle: () => {
    const next: ThemeMode = get().mode === "dark" ? "light" : "dark";
    localStorage.setItem(KEY, next);
    document.documentElement.dataset.theme = next;
    set({ mode: next });
  },
}));

// 启动即把当前模式写到 <html data-theme> 供 CSS 变量切换
document.documentElement.dataset.theme = useThemeStore.getState().mode;
