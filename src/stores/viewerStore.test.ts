import { beforeEach, describe, expect, it } from "vitest";
import { useViewerStore } from "./viewerStore";

const reset = () => useViewerStore.getState().closeAll();

describe("viewerStore tab 逻辑", () => {
  beforeEach(reset);

  it("初始无 tab", () => {
    const s = useViewerStore.getState();
    expect(s.content).toBeNull();
    expect(s.browserUrl).toBeNull();
    expect(s.terminalOpen).toBe(false);
  });

  it("openTerminal / openBrowser 开 tab 并激活", () => {
    useViewerStore.getState().openTerminal();
    expect(useViewerStore.getState().terminalOpen).toBe(true);
    expect(useViewerStore.getState().activeTab).toBe("terminal");

    useViewerStore.getState().openBrowser("http://localhost:5173");
    expect(useViewerStore.getState().browserUrl).toBe("http://localhost:5173");
    expect(useViewerStore.getState().activeTab).toBe("browser");
  });

  it("setTab 切换激活 tab", () => {
    const s = useViewerStore.getState();
    s.openTerminal();
    s.openBrowser("http://x");
    useViewerStore.getState().setTab("terminal");
    expect(useViewerStore.getState().activeTab).toBe("terminal");
  });

  it("closeTab 关闭激活 tab 时回退到其他存在的 tab", () => {
    const s = useViewerStore.getState();
    s.openTerminal(); // terminal active
    s.openBrowser("http://x"); // browser active
    // 关掉当前激活的 browser → 回退到 terminal
    useViewerStore.getState().closeTab("browser");
    expect(useViewerStore.getState().browserUrl).toBeNull();
    expect(useViewerStore.getState().activeTab).toBe("terminal");
  });

  it("closeTab 关闭非激活 tab 不改变激活 tab", () => {
    const s = useViewerStore.getState();
    s.openBrowser("http://x"); // browser active
    s.openTerminal(); // terminal active
    useViewerStore.getState().closeTab("browser"); // 关非激活
    expect(useViewerStore.getState().activeTab).toBe("terminal");
    expect(useViewerStore.getState().terminalOpen).toBe(true);
  });

  it("closeAll 清空所有 tab", () => {
    const s = useViewerStore.getState();
    s.openTerminal();
    s.openBrowser("http://x");
    useViewerStore.getState().closeAll();
    const after = useViewerStore.getState();
    expect(after.content).toBeNull();
    expect(after.browserUrl).toBeNull();
    expect(after.terminalOpen).toBe(false);
  });
});
