import { useState } from "react";
import { App, Button, Dropdown, Empty, Input, Modal } from "antd";
import { DeleteOutlined, EditOutlined, MoreOutlined, PlusOutlined } from "@ant-design/icons";
import { useChatStore } from "../../stores/chatStore";
import { useSessionStore } from "../../stores/sessionStore";

export function SessionList() {
  const { message, modal } = App.useApp();
  const { sessions, open, startNew, rename, remove } = useSessionStore();
  const currentId = useChatStore((s) => s.currentSessionId);
  const streaming = useChatStore((s) => s.streaming);
  const [renaming, setRenaming] = useState<{ id: number; title: string } | null>(null);

  const switchTo = (id: number) => {
    if (streaming) {
      message.warning("等当前回合结束后再切换会话");
      return;
    }
    if (id !== currentId) void open(id).catch((e) => message.error(String(e)));
  };

  return (
    <div className="session-list">
      <Button
        block
        icon={<PlusOutlined />}
        onClick={startNew}
        disabled={streaming}
        style={{ marginBottom: 8 }}
      >
        新建会话
      </Button>

      {sessions.length === 0 && (
        <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="还没有历史会话" />
      )}

      {sessions.map((s) => (
        <div
          key={s.id}
          className={`session-item ${s.id === currentId ? "session-item-active" : ""}`}
          onClick={() => switchTo(s.id)}
        >
          <div className="session-item-main">
            <div className="session-title">{s.title}</div>
            <div className="session-time">{s.updatedAt}</div>
          </div>
          <Dropdown
            trigger={["click"]}
            menu={{
              items: [
                { key: "rename", icon: <EditOutlined />, label: "重命名" },
                { key: "delete", icon: <DeleteOutlined />, label: "删除", danger: true },
              ],
              onClick: ({ key, domEvent }) => {
                domEvent.stopPropagation();
                if (key === "rename") setRenaming({ id: s.id, title: s.title });
                if (key === "delete") {
                  modal.confirm({
                    title: `删除会话「${s.title}」？`,
                    content: "历史消息将一并删除，不可恢复。",
                    okText: "删除",
                    okButtonProps: { danger: true },
                    cancelText: "取消",
                    onOk: () => remove(s.id).catch((e) => message.error(String(e))),
                  });
                }
              },
            }}
          >
            <Button
              type="text"
              size="small"
              icon={<MoreOutlined />}
              onClick={(e) => e.stopPropagation()}
            />
          </Dropdown>
        </div>
      ))}

      <Modal
        title="重命名会话"
        open={renaming !== null}
        onCancel={() => setRenaming(null)}
        okText="保存"
        cancelText="取消"
        onOk={() => {
          if (renaming && renaming.title.trim()) {
            void rename(renaming.id, renaming.title.trim()).catch((e) =>
              message.error(String(e)),
            );
          }
          setRenaming(null);
        }}
      >
        <Input
          value={renaming?.title ?? ""}
          onChange={(e) => setRenaming((r) => (r ? { ...r, title: e.target.value } : r))}
          onPressEnter={(e) => (e.target as HTMLInputElement).blur()}
        />
      </Modal>
    </div>
  );
}
