import { useEffect, useState } from "react";
import { Spin } from "antd";
import {
  CaretDownOutlined,
  CaretRightOutlined,
  CheckCircleOutlined,
  TeamOutlined,
} from "@ant-design/icons";
import type { ChatItem } from "../../stores/chatStore";
import { ToolCallCard } from "./ToolCallCard";

type ToolItem = Extract<ChatItem, { kind: "tool" }>;

/** 子 agent 的工具调用折叠组：运行中展开，全部完成后自动收起 */
export function SubagentGroup({ items }: { items: ToolItem[] }) {
  const running = items.some((i) => !i.done);
  const [open, setOpen] = useState(true);

  useEffect(() => {
    if (!running) setOpen(false);
  }, [running]);

  return (
    <div className={`subagent-group ${running ? "subagent-running" : ""}`}>
      <div className="subagent-header" onClick={() => setOpen((o) => !o)}>
        {open ? <CaretDownOutlined /> : <CaretRightOutlined />}
        <TeamOutlined />
        <span>子 agent 执行过程（{items.length} 步）</span>
        <span className="subagent-status">
          {running ? <Spin size="small" /> : <CheckCircleOutlined style={{ color: "#389e0d" }} />}
        </span>
      </div>
      {open && (
        <div className="subagent-body">
          {items.map((item) => (
            <ToolCallCard key={item.id} item={item} />
          ))}
        </div>
      )}
    </div>
  );
}
