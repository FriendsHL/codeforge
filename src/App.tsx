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

  // 首次启动没配 key 时直接引导到设置
  useEffect(() => {
    void hasApiKey().then((configured) => {
      if (!configured) setSettingsOpen(true);
    });
  }, []);

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
