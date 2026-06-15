import { useEffect, useMemo, useState } from "react";
import { App, Button, Dropdown, Empty, Input, Modal } from "antd";
import {
  ApiOutlined,
  DeleteOutlined,
  EditOutlined,
  FolderOpenOutlined,
  FolderOutlined,
  FormOutlined,
  MoreOutlined,
  SearchOutlined,
  ToolOutlined,
} from "@ant-design/icons";
import { listCapabilities, type Capabilities, type SessionMeta } from "../../lib/ipc";
import { useChatStore } from "../../stores/chatStore";
import { useSessionStore } from "../../stores/sessionStore";
import { useWorkspaceStore } from "../../stores/workspaceStore";

/** 相对时间，右对齐弱显（Codex 风）："刚刚/6分/3时/2天/1周/1月" */
function relativeTime(s: string): string {
  const t = new Date(s.replace(" ", "T")).getTime();
  if (Number.isNaN(t)) return s;
  const m = Math.floor((Date.now() - t) / 60000);
  if (m < 1) return "刚刚";
  if (m < 60) return `${m}分`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}时`;
  const d = Math.floor(h / 24);
  if (d < 7) return `${d}天`;
  if (d < 30) return `${Math.floor(d / 7)}周`;
  if (d < 365) return `${Math.floor(d / 30)}月`;
  return `${Math.floor(d / 365)}年`;
}

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
      <span className="session-title">{session.title}</span>
      <span className="session-time">{relativeTime(session.updatedAt)}</span>
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

/** 搜索会话弹窗 */
function SearchModal({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { sessions, open: openSession } = useSessionStore();
  const [q, setQ] = useState("");
  useEffect(() => {
    if (open) setQ("");
  }, [open]);
  const hits = useMemo(() => {
    const kw = q.trim().toLowerCase();
    const list = kw ? sessions.filter((s) => s.title.toLowerCase().includes(kw)) : sessions;
    return list.slice(0, 50);
  }, [q, sessions]);

  return (
    <Modal open={open} onCancel={onClose} footer={null} title="搜索会话" width={520}>
      <Input
        autoFocus
        prefix={<SearchOutlined />}
        placeholder="按标题搜索…"
        value={q}
        onChange={(e) => setQ(e.target.value)}
        allowClear
      />
      <div className="search-results">
        {hits.length === 0 && <div className="search-empty">没有匹配的会话</div>}
        {hits.map((s) => (
          <div
            key={s.id}
            className="search-result-item"
            onClick={() => {
              void openSession(s.id);
              onClose();
            }}
          >
            <span className="search-result-title">{s.title}</span>
            <span className="search-result-time">{s.updatedAt.slice(5, 16)}</span>
          </div>
        ))}
      </div>
    </Modal>
  );
}

/** 插件弹窗：当前可用的工具与技能 */
function PluginsModal({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [caps, setCaps] = useState<Capabilities | null>(null);
  useEffect(() => {
    if (open) {
      setCaps(null);
      void listCapabilities().then(setCaps).catch(() => setCaps({ tools: [], skills: [] }));
    }
  }, [open]);

  return (
    <Modal open={open} onCancel={onClose} footer={null} title="插件 · 工具与技能" width={560}>
      <div className="plugins-body">
        <div className="plugins-section-title">
          <ToolOutlined /> 工具 {caps ? `(${caps.tools.length})` : ""}
        </div>
        {caps?.tools.map((t) => (
          <div key={t.name} className="plugins-item">
            <span className="plugins-item-name">{t.name}</span>
            <span className="plugins-item-desc">{t.description}</span>
          </div>
        ))}
        <div className="plugins-section-title" style={{ marginTop: 16 }}>
          <ApiOutlined /> 技能 {caps ? `(${caps.skills.length})` : ""}
        </div>
        {caps && caps.skills.length === 0 && (
          <div className="search-empty">
            暂无技能。在 ~/.codeforge/skills/&lt;名&gt;/SKILL.md 添加即可被自动发现。
          </div>
        )}
        {caps?.skills.map((s) => (
          <div key={s.name} className="plugins-item">
            <span className="plugins-item-name">{s.name}</span>
            <span className="plugins-item-desc">{s.description}</span>
          </div>
        ))}
        {!caps && <div className="search-empty">加载中…</div>}
      </div>
    </Modal>
  );
}

export function ProjectsPanel() {
  const { message, modal } = App.useApp();
  const { sessions, projects, rename, startNew, forgetProject } = useSessionStore();
  const { root, openWorkspace, switchTo } = useWorkspaceStore();
  const [renaming, setRenaming] = useState<{ id: number; title: string } | null>(null);
  const [searchOpen, setSearchOpen] = useState(false);
  const [pluginsOpen, setPluginsOpen] = useState(false);

  const sessionsOf = (projectRoot: string) =>
    sessions.filter((s) => s.workspaceRoot === projectRoot);
  const looseSessions = sessions.filter(
    (s) => !s.workspaceRoot || !projects.some((p) => p.root === s.workspaceRoot),
  );

  return (
    <aside className="projects-panel">
      <nav className="side-nav">
        <button className="nav-item" onClick={startNew}>
          <FormOutlined />
          <span>新对话</span>
        </button>
        <button className="nav-item" onClick={() => setSearchOpen(true)}>
          <SearchOutlined />
          <span>搜索</span>
        </button>
        <button className="nav-item" onClick={() => setPluginsOpen(true)}>
          <ApiOutlined />
          <span>插件</span>
        </button>
        <button className="nav-item" onClick={() => void openWorkspace()}>
          <FolderOpenOutlined />
          <span>打开项目</span>
        </button>
      </nav>

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

      <SearchModal open={searchOpen} onClose={() => setSearchOpen(false)} />
      <PluginsModal open={pluginsOpen} onClose={() => setPluginsOpen(false)} />
    </aside>
  );
}
