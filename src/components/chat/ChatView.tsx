import { useEffect, useRef, useState } from "react";
import { Alert, App, Button, Empty, Input, Segmented, Select, Tag, Tooltip } from "antd";
import { ClearOutlined, SendOutlined, StopOutlined } from "@ant-design/icons";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { listAgentRoles, stopGeneration, type AgentRoleMeta } from "../../lib/ipc";
import { contextWindowFor } from "../../lib/models";
import { useChatStore } from "../../stores/chatStore";
import { fmtTokens, ringColor, toBlocks, workingLabel } from "./chatHelpers";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { ApprovalCard } from "./ApprovalCard";
import { MentionPicker } from "./MentionPicker";
import { SubagentGroup } from "./SubagentGroup";
import { ThinkingCard } from "./ThinkingCard";
import { TodoPanel } from "./TodoPanel";
import { ToolCallCard } from "./ToolCallCard";

/** 内置角色名 → 中文显示标签；自定义角色回退到原名 */
const ROLE_LABELS: Record<string, string> = {
  default: "通用",
  research: "调研",
  product: "产品方案",
  dev: "开发",
  review: "Review",
};
const roleLabel = (name: string) => ROLE_LABELS[name] ?? name;

/** 底部上下文/花费状态条：上下文占用进度 + 本轮花费 + 本会话累计花费 */
function ContextStatusBar({
  model,
  contextTokens,
  turnTokens,
  sessionTokens,
  cacheTokens,
  streaming,
}: {
  model: string;
  contextTokens: number | null;
  turnTokens: number;
  sessionTokens: number;
  cacheTokens: number;
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
      {cacheTokens > 0 && (
        <Tooltip title="本会话命中 prompt 缓存的输入 tokens，这部分按折扣计费（约为常规输入价的 1/10），命中越多越省">
          <span className="chat-status-cache">缓存命中 {fmtTokens(cacheTokens)}</span>
        </Tooltip>
      )}
    </div>
  );
}

/** 发送区旁的环形上下文占比：弧长=占比，颜色随占比加深，悬停看百分比 */
function ContextRing({ model, contextTokens }: { model: string; contextTokens: number | null }) {
  if (contextTokens === null) return null;
  const window = contextWindowFor(model);
  const used = contextTokens;
  const pct = Math.min(100, Math.round((used / window) * 100));
  const size = 28;
  const stroke = 4;
  const r = (size - stroke) / 2;
  const c = 2 * Math.PI * r;
  const dash = (pct / 100) * c;
  const color = ringColor(pct);
  return (
    <Tooltip title={`上下文占用 ${pct}%（${used.toLocaleString()} / ${window.toLocaleString()} tokens）`}>
      <svg width={size} height={size} className="ctx-ring" role="img" aria-label={`上下文占用 ${pct}%`}>
        <circle cx={size / 2} cy={size / 2} r={r} fill="none" className="ctx-ring-track" strokeWidth={stroke} />
        <circle
          cx={size / 2}
          cy={size / 2}
          r={r}
          fill="none"
          stroke={color}
          strokeWidth={stroke}
          strokeDasharray={`${dash} ${c - dash}`}
          strokeLinecap="round"
          transform={`rotate(-90 ${size / 2} ${size / 2})`}
        />
        <text x="50%" y="50%" className="ctx-ring-text" dominantBaseline="central" textAnchor="middle">
          {pct}
        </text>
      </svg>
    </Tooltip>
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
    sessionTokens,
    sessionInputTokens,
    sessionCacheTokens,
    turnInputTokens,
    turnOutputTokens,
    contextTokens,
    model,
    mode,
    setMode,
    role,
    setRole,
  } = useChatStore();
  const workspaceName = useWorkspaceStore((s) => s.name);
  const hasWorkspace = useWorkspaceStore((s) => s.root !== null);
  const { message } = App.useApp();
  const [draft, setDraft] = useState("");
  const [mentions, setMentions] = useState<string[]>([]); // @ 引用的文件
  const [mentionQuery, setMentionQuery] = useState<string | null>(null); // null=未触发
  const [roles, setRoles] = useState<AgentRoleMeta[]>([]);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight });
  }, [items]);

  // 加载可用 agent 角色（含工作区自定义）
  useEffect(() => {
    void listAgentRoles().then(setRoles).catch(() => setRoles([]));
  }, [hasWorkspace]);

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

      {(sessionTokens > 0 || sessionInputTokens > 0 || contextTokens !== null) && (
        <ContextStatusBar
          model={model}
          contextTokens={contextTokens}
          turnTokens={turnInputTokens + turnOutputTokens}
          sessionTokens={sessionInputTokens + sessionTokens}
          cacheTokens={sessionCacheTokens}
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
        <Tooltip
          title={
            role === "default"
              ? "选一个 agent 角色：用专属人设+工具集驱动整轮对话"
              : roles.find((r) => r.name === role)?.description
          }
        >
          <Select
            size="small"
            value={role}
            onChange={setRole}
            disabled={streaming}
            popupMatchSelectWidth={false}
            style={{ minWidth: 92 }}
            options={[
              { label: "通用", value: "default" },
              ...roles.map((r) => ({ label: `角色·${roleLabel(r.name)}`, value: r.name })),
            ]}
          />
        </Tooltip>
        <span className="chat-mode-hint">
          {role !== "default"
            ? `${roleLabel(role)} 角色 · 工具与职责已按角色限定`
            : mode === "ask"
              ? "写文件/命令需逐个确认"
              : mode === "auto"
                ? "自动执行，仅危险操作需确认"
                : "只读+调研，产出方案不动手"}
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
        <ContextRing model={model} contextTokens={contextTokens} />
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
