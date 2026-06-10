import { useEffect, useState } from "react";
import { App as AntdApp, Button, ConfigProvider, Tag, Tooltip } from "antd";
import { SettingOutlined } from "@ant-design/icons";
import { ChatView } from "./components/chat/ChatView";
import { SettingsModal } from "./components/settings/SettingsModal";
import { hasApiKey } from "./lib/ipc";
import { useChatStore } from "./stores/chatStore";
import "./App.css";

function App() {
  const model = useChatStore((s) => s.model);
  const [settingsOpen, setSettingsOpen] = useState(false);

  // 选了 Claude 模型但没配 key 时引导到设置（ark/xiaomi 走环境变量，无需引导）
  useEffect(() => {
    if (!model.startsWith("claude/")) return;
    void hasApiKey().then((configured) => {
      if (!configured) setSettingsOpen(true);
    });
  }, [model]);

  return (
    <ConfigProvider theme={{ token: { colorPrimary: "#d46b08" } }}>
      <AntdApp>
        <div className="app-layout">
          <header className="app-header">
            <span className="app-title">⚒️ CodeForge</span>
            <Tag color="orange">{model}</Tag>
            <div className="app-header-spacer" />
            <Tooltip title="设置">
              <Button
                type="text"
                icon={<SettingOutlined />}
                onClick={() => setSettingsOpen(true)}
              />
            </Tooltip>
          </header>
          <ChatView />
          <SettingsModal open={settingsOpen} onClose={() => setSettingsOpen(false)} />
        </div>
      </AntdApp>
    </ConfigProvider>
  );
}

export default App;
