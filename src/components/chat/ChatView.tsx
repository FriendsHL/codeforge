import { useEffect, useRef, useState } from "react";
import { Alert, Button, Empty, Input, Tooltip } from "antd";
import { ClearOutlined, SendOutlined } from "@ant-design/icons";
import ReactMarkdown from "react-markdown";
import { useChatStore } from "../../stores/chatStore";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { ToolCallCard } from "./ToolCallCard";

export function ChatView() {
  const { items, streaming, error, send, clear } = useChatStore();
  const workspaceName = useWorkspaceStore((s) => s.name);
  const [draft, setDraft] = useState("");
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight });
  }, [items]);

  const submit = () => {
    const text = draft.trim();
    if (!text || streaming) return;
    setDraft("");
    void send(text);
  };

  const emptyHint = workspaceName
    ? `已打开 ${workspaceName}，问点关于这个项目的问题试试`
    : "发个消息试试；打开项目目录后可以问代码库相关的问题";

  return (
    <div className="chat-view">
      <div className="chat-messages" ref={scrollRef}>
        {items.length === 0 && <Empty description={emptyHint} style={{ marginTop: "20vh" }} />}
        {items.map((item, index) =>
          item.kind === "tool" ? (
            <ToolCallCard key={item.id} item={item} />
          ) : (
            <div key={index} className={`chat-bubble chat-bubble-${item.role}`}>
              {item.reasoning && <div className="chat-reasoning">{item.reasoning}</div>}
              {item.role === "assistant" ? (
                <ReactMarkdown>{item.content || (item.reasoning ? "" : "…")}</ReactMarkdown>
              ) : (
                item.content
              )}
            </div>
          ),
        )}
        {error && <Alert type="error" message={error} showIcon />}
      </div>

      <div className="chat-input">
        <Input.TextArea
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onPressEnter={(e) => {
            if (!e.shiftKey) {
              e.preventDefault();
              submit();
            }
          }}
          placeholder="输入消息，Enter 发送，Shift+Enter 换行"
          autoSize={{ minRows: 1, maxRows: 6 }}
          disabled={streaming}
        />
        <Tooltip title="清空会话">
          <Button icon={<ClearOutlined />} onClick={clear} disabled={streaming} />
        </Tooltip>
        <Button type="primary" icon={<SendOutlined />} onClick={submit} loading={streaming}>
          发送
        </Button>
      </div>
    </div>
  );
}
