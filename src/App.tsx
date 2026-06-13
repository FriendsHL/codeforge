import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { App as AntdApp, Button, ConfigProvider, Tag, Tooltip, theme as antdTheme } from "antd";
import {
  BranchesOutlined,
  BulbOutlined,
  FolderOpenOutlined,
  GlobalOutlined,
  MoonOutlined,
  SettingOutlined,
} from "@ant-design/icons";
import { ChatView } from "./components/chat/ChatView";
import { ResizeHandle } from "./components/layout/ResizeHandle";
import { ProjectsPanel } from "./components/sidebar/ProjectsPanel";
import { RightPanel } from "./components/sidebar/RightPanel";
import { SettingsModal } from "./components/settings/SettingsModal";
import { ViewerPanel } from "./components/viewer/ViewerPanel";
import { hasApiKey } from "./lib/ipc";
import { useChatStore } from "./stores/chatStore";
import { useGitStore } from "./stores/gitStore";
import { useSessionStore } from "./stores/sessionStore";
import { useThemeStore } from "./stores/themeStore";
import { useTodoStore } from "./stores/todoStore";
import { LIMITS, useUiStore } from "./stores/uiStore";
import { useViewerStore } from "./stores/viewerStore";
import { useWorkspaceStore } from "./stores/workspaceStore";
import "./App.css";

function App() {
  const model = useChatStore((s) => s.model);
  const { root, name } = useWorkspaceStore();
  const branch = useGitStore((s) => s.branch);
  const viewerOpen = useViewerStore(
    (s) => s.content !== null || s.browserUrl !== null || s.terminalOpen,
  );
  const { widths, resize } = useUiStore();
  const { mode, toggle } = useThemeStore();
  const [settingsOpen, setSettingsOpen] = useState(false);

  // 启动时加载历史会话列表
  useEffect(() => {
    void useSessionStore.getState().refresh();
  }, []);

  // 选了 Claude 模型但没配 key 时引导到设置（ark/xiaomi 走环境变量，无需引导）
  useEffect(() => {
    if (!model.startsWith("claude/")) return;
    void hasApiKey().then((configured) => {
      if (!configured) setSettingsOpen(true);
    });
  }, [model]);

  // 文件 watcher：agent/用户改了文件 → 刷新 git 状态（分支、角标、改动列表）
  useEffect(() => {
    const unlisten = listen("workspace-fs-changed", () => {
      void useGitStore.getState().refresh();
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, []);

  // agent 的 browser_open 工具 → 打开浏览器面板并导航
  useEffect(() => {
    const unlisten = listen<string>("browser-open", (event) => {
      useViewerStore.getState().openBrowser(event.payload);
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, []);

  // agent 的 todo_write 工具 → 更新任务清单面板
  useEffect(() => {
    const unlisten = listen<import("./stores/todoStore").TodoItem[]>("todo-update", (event) => {
      useTodoStore.getState().set(event.payload);
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, []);

  return (
    <ConfigProvider
      theme={{
        token: { colorPrimary: "#d46b08" },
        algorithm: mode === "dark" ? antdTheme.darkAlgorithm : antdTheme.defaultAlgorithm,
      }}
    >
      <AntdApp>
        <div className="app-layout">
          <header className="app-header">
            <span className="app-title">⚒️ CodeForge</span>
            <Tag color="orange">{model}</Tag>
            {name && <Tag icon={<FolderOpenOutlined />}>{name}</Tag>}
            {branch && (
              <Tag icon={<BranchesOutlined />} color="geekblue">
                {branch}
              </Tag>
            )}
            <div className="app-header-spacer" />
            <Tooltip title={mode === "dark" ? "切到浅色" : "切到深色"}>
              <Button
                type="text"
                icon={mode === "dark" ? <BulbOutlined /> : <MoonOutlined />}
                onClick={toggle}
              />
            </Tooltip>
            <Tooltip title="浏览器面板（预览本地 dev server）">
              <Button
                type="text"
                icon={<GlobalOutlined />}
                onClick={() => {
                  const v = useViewerStore.getState();
                  if (v.browserUrl !== null) v.closeTab("browser");
                  else v.openBrowser();
                }}
              />
            </Tooltip>
            <Tooltip title="设置">
              <Button
                type="text"
                icon={<SettingOutlined />}
                onClick={() => setSettingsOpen(true)}
              />
            </Tooltip>
          </header>
          <div className="app-body">
            {/* 面板可随窗口变窄收缩到各自 min（flexShrink:1），中栏始终自适应 */}
            <div
              style={{ width: widths.left, minWidth: LIMITS.left.min, flexShrink: 1, display: "flex" }}
            >
              <ProjectsPanel />
            </div>
            <ResizeHandle onDelta={(dx) => resize("left", dx, true)} />
            <ChatView />
            {viewerOpen && (
              <>
                <ResizeHandle onDelta={(dx) => resize("viewer", dx, false)} />
                <div
                  style={{
                    width: widths.viewer,
                    minWidth: LIMITS.viewer.min,
                    flexShrink: 1,
                    display: "flex",
                  }}
                >
                  <ViewerPanel />
                </div>
              </>
            )}
            {root && (
              <>
                <ResizeHandle onDelta={(dx) => resize("right", dx, false)} />
                <div
                  style={{
                    width: widths.right,
                    minWidth: LIMITS.right.min,
                    flexShrink: 1,
                    display: "flex",
                  }}
                >
                  <RightPanel />
                </div>
              </>
            )}
          </div>
          <SettingsModal open={settingsOpen} onClose={() => setSettingsOpen(false)} />
        </div>
      </AntdApp>
    </ConfigProvider>
  );
}

export default App;
