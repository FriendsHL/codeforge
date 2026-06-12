import { create } from "zustand";

// 三个可调宽面板：左（项目/会话）、查看器、右（文件/改动）。中栏 flex 自适应。
const LIMITS = {
  left: { min: 180, max: 420, default: 240 },
  viewer: { min: 320, max: 1000, default: 520 },
  right: { min: 200, max: 520, default: 280 },
} as const;

export type PanelKey = keyof typeof LIMITS;

function load(key: PanelKey): number {
  const saved = Number(localStorage.getItem(`codeforge.width.${key}`));
  const { min, max, default: def } = LIMITS[key];
  return saved >= min && saved <= max ? saved : def;
}

interface UiState {
  widths: Record<PanelKey, number>;
  /** delta 为鼠标横向位移；grow 表示向右拖时该面板变宽还是变窄 */
  resize: (panel: PanelKey, delta: number, grow: boolean) => void;
}

export const useUiStore = create<UiState>((set) => ({
  widths: { left: load("left"), viewer: load("viewer"), right: load("right") },

  resize: (panel, delta, grow) =>
    set((state) => {
      const { min, max } = LIMITS[panel];
      const next = Math.min(
        max,
        Math.max(min, state.widths[panel] + (grow ? delta : -delta)),
      );
      localStorage.setItem(`codeforge.width.${panel}`, String(next));
      return { widths: { ...state.widths, [panel]: next } };
    }),
}));
