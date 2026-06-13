import { useState } from "react";
import {
  CaretDownOutlined,
  CaretRightOutlined,
  CheckCircleFilled,
  CloseCircleFilled,
  LoadingOutlined,
} from "@ant-design/icons";
import { Tag, Tooltip } from "antd";
import { useTeamStore } from "../../stores/teamStore";

const ROLE_LABEL: Record<string, string> = {
  research: "调研",
  product: "产品方案",
  dev: "开发",
  review: "Review",
  test: "测试",
};

/** 异步团队任务看板：spawn_team 派发的后台 agent 进度，随 TeamUpdate 实时刷新 */
export function TeamPanel() {
  const tasks = useTeamStore((s) => s.tasks);
  const [open, setOpen] = useState(true);
  if (tasks.length === 0) return null;

  const running = tasks.filter((t) => t.status === "running").length;

  return (
    <div className="team-panel">
      <div className="team-header" onClick={() => setOpen((o) => !o)}>
        {open ? <CaretDownOutlined /> : <CaretRightOutlined />}
        <span>团队任务</span>
        <span className="team-progress">
          {tasks.length - running}/{tasks.length} 完成
        </span>
      </div>
      {open && (
        <div className="team-body">
          {tasks.map((t) => (
            <div key={t.id} className="team-item">
              {t.status === "running" ? (
                <LoadingOutlined style={{ color: "#d46b08" }} />
              ) : t.status === "done" ? (
                <CheckCircleFilled style={{ color: "#389e0d" }} />
              ) : (
                <CloseCircleFilled style={{ color: "#cf1322" }} />
              )}
              <span className="team-item-title">{t.title}</span>
              {t.role && <Tag>{ROLE_LABEL[t.role] ?? t.role}</Tag>}
              {t.result && t.status !== "running" && (
                <Tooltip title={t.result.slice(0, 600)}>
                  <span className="team-item-result">查看结果</span>
                </Tooltip>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
