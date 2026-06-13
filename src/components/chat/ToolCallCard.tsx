import { App, Button, Collapse, Spin, Tag } from "antd";
import {
  CheckCircleOutlined,
  CloseCircleOutlined,
  ToolOutlined,
  UndoOutlined,
} from "@ant-design/icons";
import { revertCheckpoint } from "../../lib/ipc";
import type { ChatItem } from "../../stores/chatStore";
import { useChatStore } from "../../stores/chatStore";

const TOOL_LABELS: Record<string, string> = {
  read_file: "读文件",
  list_dir: "列目录",
  glob: "找文件",
  grep: "搜内容",
  git_status: "git 状态",
  git_diff: "git diff",
  git_log: "git 历史",
  write_file: "写文件",
  edit_file: "改文件",
  bash: "执行命令",
  web_fetch: "抓网页",
  web_search: "搜索",
  research_plan: "调研方法",
  spawn_subagents: "子agent团队",
  browser_open: "打开浏览器",
  todo_write: "更新任务清单",
  remember: "记住",
};

function summarizeInput(name: string, input: unknown): string {
  if (typeof input !== "object" || input === null) return "";
  const obj = input as Record<string, unknown>;
  switch (name) {
    case "read_file":
    case "write_file":
    case "edit_file":
      return String(obj.path ?? "");
    case "list_dir":
      return String(obj.path ?? ".");
    case "glob":
      return String(obj.pattern ?? "");
    case "grep":
      return [obj.pattern, obj.include, obj.path].filter(Boolean).join("  ");
    case "bash":
      return String(obj.command ?? "");
    case "web_fetch":
    case "browser_open":
      return String(obj.url ?? "");
    case "web_search":
      return String(obj.query ?? "");
    case "research_plan":
      return String(obj.question ?? "");
    case "spawn_subagents": {
      const tasks = obj.tasks as { title?: string }[] | undefined;
      return (tasks ?? []).map((t) => t.title).filter(Boolean).join(" | ");
    }
    case "remember":
      return String(obj.content ?? "");
    case "git_status":
      return "";
    case "git_diff":
    case "git_log":
      return String(obj.path ?? "");
    default:
      return JSON.stringify(obj);
  }
}

export function ToolCallCard({ item }: { item: Extract<ChatItem, { kind: "tool" }> }) {
  const { message, modal } = App.useApp();
  const status = !item.done ? (
    <Spin size="small" />
  ) : item.isError ? (
    <CloseCircleOutlined style={{ color: "#cf1322" }} />
  ) : (
    <CheckCircleOutlined style={{ color: "#389e0d" }} />
  );

  const duration =
    item.done && item.durationMs !== undefined
      ? item.durationMs >= 1000
        ? `${(item.durationMs / 1000).toFixed(1)}s`
        : `${item.durationMs}ms`
      : null;

  const revert = (e: React.MouseEvent) => {
    e.stopPropagation();
    const sessionId = useChatStore.getState().currentSessionId;
    if (sessionId === null || !item.checkpointId) return;
    modal.confirm({
      title: "回滚这次文件改动？",
      content: `把「${summarizeInput(item.name, item.input)}」改动的文件恢复到改动前。`,
      okText: "回滚",
      okButtonProps: { danger: true },
      cancelText: "取消",
      onOk: async () => {
        try {
          const msg = await revertCheckpoint(sessionId, item.checkpointId!);
          message.success(msg);
          useChatStore.setState((s) => ({
            items: s.items.map((i) =>
              i.kind === "tool" && i.id === item.id ? { ...i, reverted: true } : i,
            ),
          }));
        } catch (err) {
          message.error(String(err));
        }
      },
    });
  };

  const canRevert = item.done && !item.isError && item.checkpointId && !item.reverted;

  return (
    <Collapse
      size="small"
      className="tool-card"
      items={[
        {
          key: item.id,
          label: (
            <span className="tool-card-label">
              <ToolOutlined />
              <Tag>{TOOL_LABELS[item.name] ?? item.name}</Tag>
              <code className="tool-card-summary">{summarizeInput(item.name, item.input)}</code>
              {duration && <span className="tool-card-duration">{duration}</span>}
              {item.reverted && <Tag color="default">已回滚</Tag>}
              {canRevert && (
                <Button
                  type="text"
                  size="small"
                  icon={<UndoOutlined />}
                  onClick={revert}
                  className="tool-card-revert"
                >
                  回滚
                </Button>
              )}
              {status}
            </span>
          ),
          children: (
            <pre className="tool-card-output">
              {item.output ?? "执行中…"}
            </pre>
          ),
        },
      ]}
    />
  );
}
