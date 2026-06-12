import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { App as AntdApp, Button, ConfigProvider, Tag, Tooltip } from "antd";
import {
  BranchesOutlined,
  FolderOpenOutlined,
  GlobalOutlined,
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
import { LIMITS, useUiStore } from "./stores/uiStore";
import { useViewerStore } from "./stores/viewerStore";
import { useWorkspaceStore } from "./stores/workspaceStore";
import "./App.css";

function App() {
  const model = useChatStore((s) => s.model);
  const { root, name } = useWorkspaceStore();
  const branch = useGitStore((s) => s.branch);
  const viewerOpen = useViewerStore((s) => s.content !== null);
  const { widths, resize } = useUiStore();
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

  return (
    <ConfigProvider theme={{ token: { colorPrimary: "#d46b08" } }}>
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
            <Tooltip title="浏览器面板（预览本地 dev server）">
              <Button
                type="text"
                icon={<GlobalOutlined />}
                onClick={() => {
                  const { content, openBrowser, close } = useViewerStore.getState();
                  if (content?.type === "browser") close();
                  else openBrowser();
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
