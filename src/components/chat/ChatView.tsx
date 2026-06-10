import { useEffect, useRef, useState } from "react";
import { Alert, Button, Empty, Input, Tooltip } from "antd";
import { ClearOutlined, SendOutlined } from "@ant-design/icons";
import ReactMarkdown from "react-markdown";
import { useChatStore } from "../../stores/chatStore";

export function ChatView() {
  const { messages, streaming, error, send, clear } = useChatStore();
  const [draft, setDraft] = useState("");
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight });
  }, [messages]);

  const submit = () => {
    const text = draft.trim();
    if (!text || streaming) return;
    setDraft("");
    void send(text);
  };

  return (
    <div className="chat-view">
      <div className="chat-messages" ref={scrollRef}>
        {messages.length === 0 && (
          <Empty description="发个消息试试，比如：你好" style={{ marginTop: "20vh" }} />
        )}
        {messages.map((message, index) => (
          <div key={index} className={`chat-bubble chat-bubble-${message.role}`}>
            {message.role === "assistant" ? (
              <ReactMarkdown>{message.content || "…"}</ReactMarkdown>
            ) : (
              message.content
            )}
          </div>
        ))}
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
        <Button
          type="primary"
          icon={<SendOutlined />}
          onClick={submit}
          loading={streaming}
        >
          发送
        </Button>
      </div>
    </div>
  );
}
