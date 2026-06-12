import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { App as AntdApp, Button, ConfigProvider, Tag, Tooltip } from "antd";
import { BranchesOutlined, FolderOpenOutlined, SettingOutlined } from "@ant-design/icons";
import { ChatView } from "./components/chat/ChatView";
import { ProjectsPanel } from "./components/sidebar/ProjectsPanel";
import { RightPanel } from "./components/sidebar/RightPanel";
import { SettingsModal } from "./components/settings/SettingsModal";
import { ViewerPanel } from "./components/viewer/ViewerPanel";
import { hasApiKey } from "./lib/ipc";
import { useChatStore } from "./stores/chatStore";
import { useGitStore } from "./stores/gitStore";
import { useSessionStore } from "./stores/sessionStore";
import { useWorkspaceStore } from "./stores/workspaceStore";
import "./App.css";

function App() {
  const model = useChatStore((s) => s.model);
  const { root, name } = useWorkspaceStore();
  const branch = useGitStore((s) => s.branch);
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
            <Tooltip title="设置">
              <Button
                type="text"
                icon={<SettingOutlined />}
                onClick={() => setSettingsOpen(true)}
              />
            </Tooltip>
          </header>
          <div className="app-body">
            <ProjectsPanel />
            <ChatView />
            {/* 文件/diff 查看器（将来浏览器面板也在这个位置） */}
            <ViewerPanel />
            {root && <RightPanel />}
          </div>
          <SettingsModal open={settingsOpen} onClose={() => setSettingsOpen(false)} />
        </div>
      </AntdApp>
    </ConfigProvider>
  );
}

export default App;
