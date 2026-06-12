import { useState } from "react";
import { Badge, Segmented } from "antd";
import { useGitStore } from "../../stores/gitStore";
import { Explorer } from "../explorer/Explorer";

/** 右栏：当前项目的文件树 + 改动列表 */
export function RightPanel() {
  const [view, setView] = useState<"files" | "changes">("files");
  const changeCount = useGitStore((s) => s.changes.length);

  return (
    <aside className="right-panel">
      <div className="explorer-switch">
        <Segmented
          block
          size="small"
          value={view}
          onChange={(v) => setView(v as "files" | "changes")}
          options={[
            { label: "文件", value: "files" },
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
            },
          ]}
        />
      </div>
      <Explorer view={view} />
    </aside>
  );
}
