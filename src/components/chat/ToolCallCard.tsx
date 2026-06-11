import { Collapse, Spin, Tag } from "antd";
import {
  CheckCircleOutlined,
  CloseCircleOutlined,
  ToolOutlined,
} from "@ant-design/icons";
import type { ChatItem } from "../../stores/chatStore";

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
      return String(obj.url ?? "");
    case "web_search":
      return String(obj.query ?? "");
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
  const status = !item.done ? (
    <Spin size="small" />
  ) : item.isError ? (
    <CloseCircleOutlined style={{ color: "#cf1322" }} />
  ) : (
    <CheckCircleOutlined style={{ color: "#389e0d" }} />
  );

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
