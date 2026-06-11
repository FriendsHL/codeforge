import { useState } from "react";
import { App, Button, Space, Tag } from "antd";
import {
  CheckOutlined,
  CloseOutlined,
  EditOutlined,
  ThunderboltOutlined,
} from "@ant-design/icons";
import { approvePermission } from "../../lib/ipc";
import type { ChatItem } from "../../stores/chatStore";
import { useChatStore } from "../../stores/chatStore";
import { DiffView } from "../explorer/DiffView";

export function ApprovalCard({ item }: { item: Extract<ChatItem, { kind: "approval" }> }) {
  const { message } = App.useApp();
  const [submitting, setSubmitting] = useState(false);

  const decide = async (approved: boolean, allowAll: boolean) => {
    setSubmitting(true);
    try {
      await approvePermission(item.requestId, approved, allowAll);
      const decision = allowAll ? "allowAll" : approved ? "approved" : "denied";
      useChatStore.setState((state) => ({
        items: state.items.map((i) =>
          i.kind === "approval" && i.requestId === item.requestId ? { ...i, decision } : i,
        ),
      }));
    } catch (e) {
      message.error(String(e));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="approval-card">
      <div className="approval-header">
        <EditOutlined />
        <span>
          agent 请求{item.toolName === "write_file" ? "写入" : "修改"}
        </span>
        <Tag color="orange">{item.path}</Tag>
        {item.decision === "approved" && <Tag color="green">已允许</Tag>}
        {item.decision === "allowAll" && <Tag color="green">已允许（本会话全部）</Tag>}
        {item.decision === "denied" && <Tag color="red">已拒绝</Tag>}
      </div>
      <DiffView path={item.path} diff={item.diff} />
      {!item.decision && (
        <Space className="approval-actions">
          <Button
            danger
            icon={<CloseOutlined />}
            disabled={submitting}
            onClick={() => void decide(false, false)}
          >
            拒绝
          </Button>
          <Button
            type="primary"
            icon={<CheckOutlined />}
            disabled={submitting}
            onClick={() => void decide(true, false)}
          >
            允许
          </Button>
          <Button
            icon={<ThunderboltOutlined />}
            disabled={submitting}
            onClick={() => void decide(true, true)}
          >
            本会话全部允许
          </Button>
        </Space>
      )}
    </div>
  );
}
