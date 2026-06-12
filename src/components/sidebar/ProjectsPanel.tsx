import { useState } from "react";
import { App, Button, Dropdown, Empty, Input, Modal, Tooltip } from "antd";
import {
  DeleteOutlined,
  EditOutlined,
  FolderOpenOutlined,
  FolderOutlined,
  MoreOutlined,
  PlusOutlined,
} from "@ant-design/icons";
import type { SessionMeta } from "../../lib/ipc";
import { useChatStore } from "../../stores/chatStore";
import { useSessionStore } from "../../stores/sessionStore";
import { useWorkspaceStore } from "../../stores/workspaceStore";

function SessionRow({
  session,
  onRename,
}: {
  session: SessionMeta;
  onRename: (s: { id: number; title: string }) => void;
}) {
  const { message, modal } = App.useApp();
  const { open, remove } = useSessionStore();
  const currentId = useChatStore((s) => s.currentSessionId);
  const streaming = useChatStore((s) => s.streaming);

  const switchTo = () => {
    if (streaming) {
      message.warning("等当前回合结束后再切换会话");
      return;
    }
    if (session.id !== currentId) {
      void open(session.id).catch((e) => message.error(String(e)));
    }
  };

  return (
    <div
      className={`session-item ${session.id === currentId ? "session-item-active" : ""}`}
      onClick={switchTo}
    >
      <div className="session-item-main">
        <div className="session-title">{session.title}</div>
        <div className="session-time">{session.updatedAt}</div>
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
            if (key === "rename") onRename({ id: session.id, title: session.title });
            if (key === "delete") {
              modal.confirm({
                title: `删除会话「${session.title}」？`,
                content: "历史消息将一并删除，不可恢复。",
                okText: "删除",
                okButtonProps: { danger: true },
                cancelText: "取消",
                onOk: () => remove(session.id).catch((e) => message.error(String(e))),
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
  );
}

export function ProjectsPanel() {
  const { message, modal } = App.useApp();
  const { sessions, projects, rename, startNew, forgetProject } = useSessionStore();
  const { root, openWorkspace, switchTo } = useWorkspaceStore();
  const streaming = useChatStore((s) => s.streaming);
  const [renaming, setRenaming] = useState<{ id: number; title: string } | null>(null);

  const sessionsOf = (projectRoot: string) =>
    sessions.filter((s) => s.workspaceRoot === projectRoot);
  const looseSessions = sessions.filter(
    (s) => !s.workspaceRoot || !projects.some((p) => p.root === s.workspaceRoot),
  );

  return (
    <aside className="projects-panel">
      <div className="projects-actions">
        <Button size="small" icon={<FolderOpenOutlined />} onClick={() => void openWorkspace()}>
          打开项目
        </Button>
        <Tooltip title="在当前项目下开新会话">
          <Button size="small" icon={<PlusOutlined />} onClick={startNew} disabled={streaming}>
            新会话
          </Button>
        </Tooltip>
      </div>

      <div className="projects-scroll">
        {projects.length === 0 && looseSessions.length === 0 && (
          <Empty
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description="打开一个项目目录开始"
            style={{ marginTop: 32 }}
          />
        )}

        {projects.map((p) => (
          <div key={p.root} className="project-group">
            <div
              className={`project-header ${p.root === root ? "project-header-active" : ""}`}
              onClick={() => void switchTo(p.root).catch((e) => message.error(String(e)))}
              title={p.root}
            >
              <FolderOutlined />
              <span className="project-name">{p.name}</span>
              <Dropdown
                trigger={["click"]}
                menu={{
                  items: [
                    { key: "forget", icon: <DeleteOutlined />, label: "从列表移除" },
                  ],
                  onClick: ({ key, domEvent }) => {
                    domEvent.stopPropagation();
                    if (key === "forget") {
                      modal.confirm({
                        title: `从最近项目移除「${p.name}」？`,
                        content: "不会删除磁盘文件，其会话会移到「其他会话」。",
                        okText: "移除",
                        cancelText: "取消",
                        onOk: () => forgetProject(p.root),
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
            {sessionsOf(p.root).map((s) => (
              <SessionRow key={s.id} session={s} onRename={setRenaming} />
            ))}
          </div>
        ))}

        {looseSessions.length > 0 && (
          <div className="project-group">
            <div className="project-header project-header-loose">其他会话</div>
            {looseSessions.map((s) => (
              <SessionRow key={s.id} session={s} onRename={setRenaming} />
            ))}
          </div>
        )}
      </div>

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
        />
      </Modal>
    </aside>
  );
}
