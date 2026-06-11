import { useState } from "react";
import { Badge, Empty, Segmented } from "antd";
import { useGitStore } from "../../stores/gitStore";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { Explorer } from "../explorer/Explorer";
import { SessionList } from "./SessionList";

type View = "sessions" | "files" | "changes";

export function Sidebar() {
  const [view, setView] = useState<View>("sessions");
  const root = useWorkspaceStore((s) => s.root);
  const changeCount = useGitStore((s) => s.changes.length);

  return (
    <aside className="app-sider">
      <div className="explorer-switch">
        <Segmented
          block
          size="small"
          value={view}
          onChange={(v) => setView(v as View)}
          options={[
            { label: "会话", value: "sessions" },
            { label: "文件", value: "files", disabled: !root },
            {
              label: (
                <span>
                  改动
                  <Badge
                    count={changeCount}
                    size="small"
                    color="#d46b08"
                    style={{ marginLeft: 4 }}
                  />
                </span>
              ),
              value: "changes",
              disabled: !root,
            },
          ]}
        />
      </div>
      {view === "sessions" ? (
        <SessionList />
      ) : root ? (
        <Explorer view={view} />
      ) : (
        <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="先打开一个项目" />
      )}
    </aside>
  );
}
