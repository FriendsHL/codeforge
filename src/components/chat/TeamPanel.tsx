import { useState } from "react";
import {
  CaretDownOutlined,
  CaretRightOutlined,
  CheckCircleFilled,
  CloseCircleFilled,
  LoadingOutlined,
  MinusCircleFilled,
} from "@ant-design/icons";
import { Tag } from "antd";
import { useTeamStore, type TeamTask } from "../../stores/teamStore";

const ROLE_LABEL: Record<string, string> = {
  research: "调研",
  product: "产品方案",
  dev: "开发",
  review: "Review",
  test: "测试",
};

function StatusIcon({ status }: { status: TeamTask["status"] }) {
  if (status === "running") return <LoadingOutlined style={{ color: "#1677ff" }} />;
  if (status === "done") return <CheckCircleFilled style={{ color: "#389e0d" }} />;
  if (status === "cancelled") return <MinusCircleFilled style={{ color: "#8c8c8c" }} />;
  return <CloseCircleFilled style={{ color: "#cf1322" }} />;
}

const STATUS_TEXT: Record<TeamTask["status"], string> = {
  running: "运行中",
  done: "已完成",
  failed: "失败",
  cancelled: "已取消",
};

/** 异步团队任务看板：spawn_team 派发的后台 agent，点开看每个 agent 的产出 */
export function TeamPanel() {
  const tasks = useTeamStore((s) => s.tasks);
  const [open, setOpen] = useState(true);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  if (tasks.length === 0) return null;

  const running = tasks.filter((t) => t.status === "running").length;
  const toggle = (id: string) =>
    setExpanded((prev) => {
      const next = new Set(prev);
      next.has(id) ? next.delete(id) : next.add(id);
      return next;
    });

  return (
    <div className="team-panel">
      <div className="team-header" onClick={() => setOpen((o) => !o)}>
        {open ? <CaretDownOutlined /> : <CaretRightOutlined />}
        <span>团队 · {tasks.length} 个 agent</span>
        <span className="team-progress">
          {running > 0 ? `${running} 运行中` : "全部完成"}
        </span>
      </div>
      {open && (
        <div className="team-body">
          {tasks.map((t) => {
            const isOpen = expanded.has(t.id);
            const canExpand = !!t.result;
            return (
              <div key={t.id} className={`team-task${isOpen ? " open" : ""}`}>
                <div
                  className={`team-task-row${canExpand ? " clickable" : ""}`}
                  onClick={() => canExpand && toggle(t.id)}
                >
                  <StatusIcon status={t.status} />
                  <span className="team-task-id">{t.id}</span>
                  {t.role && <Tag className="team-role">{ROLE_LABEL[t.role] ?? t.role}</Tag>}
                  <span className="team-task-title">{t.title}</span>
                  <span className={`team-task-status team-status-${t.status}`}>
                    {STATUS_TEXT[t.status]}
                  </span>
                  {canExpand &&
                    (isOpen ? <CaretDownOutlined /> : <CaretRightOutlined />)}
                </div>
                {isOpen && t.result && (
                  <div className="team-task-result">{t.result}</div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
