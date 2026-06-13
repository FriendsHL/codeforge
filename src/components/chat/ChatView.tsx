import { useEffect, useRef, useState } from "react";
import { Alert, App, Button, Empty, Input, Segmented, Tag, Tooltip } from "antd";
import { ClearOutlined, SendOutlined, StopOutlined } from "@ant-design/icons";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { stopGeneration } from "../../lib/ipc";
import { contextWindowFor } from "../../lib/models";
import { ChatItem, useChatStore } from "../../stores/chatStore";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { ApprovalCard } from "./ApprovalCard";
import { MentionPicker } from "./MentionPicker";
import { SubagentGroup } from "./SubagentGroup";
import { ThinkingCard } from "./ThinkingCard";
import { TodoPanel } from "./TodoPanel";
import { ToolCallCard } from "./ToolCallCard";
import { TerminalPanel } from "../terminal/TerminalPanel";

type ToolItem = Extract<ChatItem, { kind: "tool" }>;

/** 子 agent 的工具调用（事件 id 带 -sN- 前缀）；连续的合并为一个折叠组 */
function isSubagentTool(item: ChatItem): item is ToolItem {
  return item.kind === "tool" && /-s\d+-/.test(item.id);
}

type RenderBlock =
  | { type: "item"; item: ChatItem; index: number }
  | { type: "subagents"; items: ToolItem[]; key: string };

function toBlocks(items: ChatItem[]): RenderBlock[] {
  const blocks: RenderBlock[] = [];
  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    if (isSubagentTool(item)) {
      const last = blocks[blocks.length - 1];
      if (last?.type === "subagents") {
        last.items.push(item);
      } else {
        blocks.push({ type: "subagents", items: [item], key: item.id });
      }
    } else {
      blocks.push({ type: "item", item, index: i });
    }
  }
  return blocks;
}

/** 流式期间的状态指示：根据最后一个条目推断 agent 正在干什么 */
function workingLabel(items: ChatItem[]): string {
  const last = items[items.length - 1];
  if (last?.kind === "approval" && !last.decision) return "等待你的审批";
  if (last?.kind === "tool" && !last.done) return "执行工具中";
  if (last?.kind === "msg" && last.role === "assistant" && last.content) return "回答中";
  return "思考中";
}

/** 把 tokens 数压成易读的 1.2k / 34.5k 形式 */
function fmtTokens(n: number): string {
  if (n < 1000) return `${n}`;
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`;
  return `${(n / 1_000_000).toFixed(2)}M`;
}

/** 底部上下文/花费状态条：上下文占用进度 + 本轮花费 + 本会话累计花费 */
function ContextStatusBar({
  model,
  contextTokens,
  turnTokens,
  sessionTokens,
  streaming,
}: {
  model: string;
  contextTokens: number | null;
  turnTokens: number;
  sessionTokens: number;
  streaming: boolean;
}) {
  const window = contextWindowFor(model);
  const used = contextTokens ?? 0;
  const pct = Math.min(100, Math.round((used / window) * 100));
  // 70% 以下绿色、70~90% 橙色、90%+ 红色，提示该 /compact 了
  const color = pct >= 90 ? "#ff4d4f" : pct >= 70 ? "#fa8c16" : "#52c41a";

  return (
    <div className="chat-status">
      {contextTokens !== null && (
        <Tooltip title={`当前上下文 ${used.toLocaleString()} / ${window.toLocaleString()} tokens（模型窗口）。接近上限时用 /compact 压缩`}>
          <span className="chat-status-ctx">
            <span className="chat-status-bar">
              <span className="chat-status-bar-fill" style={{ width: `${pct}%`, background: color }} />
            </span>
            上下文 {fmtTokens(used)} / {fmtTokens(window)}（{pct}%）
          </span>
        </Tooltip>
      )}
      <Tooltip title="本轮（最近一次提问）所有 LLM 调用的输入+输出 tokens 之和">
        <span>本轮 {fmtTokens(turnTokens)} tokens{streaming ? " …" : ""}</span>
      </Tooltip>
      <Tooltip title="本会话至今所有 LLM 调用累计的输入+输出 tokens（约等于计费量）">
        <span>本会话累计 {fmtTokens(sessionTokens)} tokens</span>
      </Tooltip>
    </div>
  );
}

export function ChatView() {
  const {
    items,
    streaming,
    error,
    send,
    queue,
    clear,
    terminalOpen,
    sessionTokens,
    sessionInputTokens,
    turnInputTokens,
    turnOutputTokens,
    contextTokens,
    model,
    mode,
    setMode,
  } = useChatStore();
  const workspaceName = useWorkspaceStore((s) => s.name);
  const hasWorkspace = useWorkspaceStore((s) => s.root !== null);
  const { message } = App.useApp();
  const [draft, setDraft] = useState("");
  const [mentions, setMentions] = useState<string[]>([]); // @ 引用的文件
  const [mentionQuery, setMentionQuery] = useState<string | null>(null); // null=未触发
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight });
  }, [items]);

  const submit = () => {
    const text = draft.trim();
    if (!text) return;
    setDraft("");
    setMentionQuery(null);
    if (streaming) {
      // 生成中：排队追加；万一回合刚好结束（排队失败）则转为正常发送
      void queue(text).then((ok) => {
        if (!ok) void send(text, []);
      });
      return;
    }
    const used = mentions;
    setMentions([]);
    void send(text, used);
  };

  // 监听输入：光标处刚打 @ 且后面是连续非空白 → 进入引用模式
  const onDraftChange = (value: string) => {
    setDraft(value);
    if (!hasWorkspace) return;
    const m = /@([^\s@]*)$/.exec(value);
    setMentionQuery(m ? m[1] : null);
  };

  const pickMention = (path: string) => {
    if (!mentions.includes(path)) setMentions([...mentions, path]);
    // 去掉输入框里正在打的 @query 片段
    setDraft((d) => d.replace(/@[^\s@]*$/, ""));
    setMentionQuery(null);
  };

  const stop = () => {
    void stopGeneration().catch((e) => message.error(String(e)));
  };

  const emptyHint = workspaceName
    ? `已打开 ${workspaceName}，问点关于这个项目的问题试试`
    : "发个消息试试；打开项目目录后可以问代码库相关的问题";

  return (
    <div className="chat-view">
      <div className="chat-messages" ref={scrollRef}>
        {items.length === 0 && <Empty description={emptyHint} style={{ marginTop: "20vh" }} />}
        {toBlocks(items).map((block) => {
          if (block.type === "subagents") {
            return <SubagentGroup key={block.key} items={block.items} />;
          }
          const { item, index } = block;
          if (item.kind === "tool") return <ToolCallCard key={item.id} item={item} />;
          if (item.kind === "approval") return <ApprovalCard key={item.requestId} item={item} />;
          if (item.kind === "notice") {
            return (
              <div key={index} className="chat-notice">
                {item.text}
              </div>
            );
          }
          const isLast = index === items.length - 1;
          const reasoningLive =
            streaming && isLast && item.role === "assistant" && !item.content;
          return (
            <div key={index} className={`chat-bubble chat-bubble-${item.role}`}>
              {item.reasoning && (
                <ThinkingCard text={item.reasoning} live={reasoningLive} />
              )}
              {item.role === "assistant" ? (
                <ReactMarkdown remarkPlugins={[remarkGfm]}>
                  {item.content || ""}
                </ReactMarkdown>
              ) : (
                item.content
              )}
              {item.queued && <span className="bubble-queued">排队中…</span>}
              {item.mentions && item.mentions.length > 0 && (
                <div className="bubble-mentions">
                  {item.mentions.map((m) => (
                    <span key={m} className="bubble-mention">@{m}</span>
                  ))}
                </div>
              )}
            </div>
          );
        })}

        {streaming && (
          <div className="working-row">
            <span className="working-dots">
              <span /><span /><span />
            </span>
            {workingLabel(items)}
          </div>
        )}

        {error && <Alert type="error" message={error} showIcon />}
      </div>

      <TodoPanel />

      {terminalOpen && <TerminalPanel />}

      {(sessionTokens > 0 || sessionInputTokens > 0 || contextTokens !== null) && (
        <ContextStatusBar
          model={model}
          contextTokens={contextTokens}
          turnTokens={turnInputTokens + turnOutputTokens}
          sessionTokens={sessionInputTokens + sessionTokens}
          streaming={streaming}
        />
      )}

      {mentions.length > 0 && (
        <div className="mention-chips">
          {mentions.map((m) => (
            <Tag
              key={m}
              closable
              onClose={() => setMentions(mentions.filter((x) => x !== m))}
              color="orange"
            >
              @{m}
            </Tag>
          ))}
        </div>
      )}

      <div className="chat-mode-bar">
        <Segmented
          size="small"
          value={mode}
          onChange={(v) => setMode(v as "ask" | "auto" | "plan")}
          disabled={streaming}
          options={[
            { label: "询问", value: "ask" },
            { label: "自动", value: "auto" },
            { label: "计划", value: "plan" },
          ]}
        />
        <span className="chat-mode-hint">
          {mode === "ask" && "写文件/命令需逐个确认"}
          {mode === "auto" && "自动执行，仅危险操作需确认"}
          {mode === "plan" && "只读+调研，产出方案不动手"}
        </span>
      </div>

      <div className="chat-input">
        <div className="chat-input-box">
          {mentionQuery !== null && (
            <MentionPicker
              query={mentionQuery}
              onPick={pickMention}
              onClose={() => setMentionQuery(null)}
            />
          )}
          <Input.TextArea
            value={draft}
            onChange={(e) => onDraftChange(e.target.value)}
            onPressEnter={(e) => {
              // 引用浮层打开时 Enter 交给浮层选择，不发送
              if (mentionQuery !== null) return;
              if (!e.shiftKey) {
                e.preventDefault();
                submit();
              }
            }}
            placeholder={
              streaming
                ? "生成中…可继续输入，Enter 追加到对话"
                : hasWorkspace
                  ? "输入消息，Enter 发送；@ 引用文件；/help 查看快捷命令"
                  : "输入消息，Enter 发送；/help 查看快捷命令"
            }
            autoSize={{ minRows: 1, maxRows: 6 }}
          />
        </div>
        <Tooltip title="清空会话">
          <Button icon={<ClearOutlined />} onClick={clear} disabled={streaming} />
        </Tooltip>
        {streaming ? (
          <>
            <Tooltip title="追加到当前对话">
              <Button icon={<SendOutlined />} onClick={submit} />
            </Tooltip>
            <Button danger type="primary" icon={<StopOutlined />} onClick={stop}>
              停止
            </Button>
          </>
        ) : (
          <Button type="primary" icon={<SendOutlined />} onClick={submit}>
            发送
          </Button>
        )}
      </div>
    </div>
  );
}
