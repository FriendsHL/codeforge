// ChatView 的纯函数（无 antd/React 依赖，便于单测）。
import type { ChatItem } from "../../stores/chatStore";

export type ToolItem = Extract<ChatItem, { kind: "tool" }>;

export type RenderBlock =
  | { type: "item"; item: ChatItem; index: number }
  | { type: "subagents"; items: ToolItem[]; key: string };

/** 子 agent 的工具调用（事件 id 带 -sN- 前缀） */
export function isSubagentTool(item: ChatItem): item is ToolItem {
  return item.kind === "tool" && /-s\d+-/.test(item.id);
}

/** 把消息流切成渲染块：连续的子 agent 工具调用合并成一个折叠组 */
export function toBlocks(items: ChatItem[]): RenderBlock[] {
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
export function workingLabel(items: ChatItem[]): string {
  const last = items[items.length - 1];
  if (last?.kind === "approval" && !last.decision) return "等待你的审批";
  if (last?.kind === "tool" && !last.done) return "执行工具中";
  if (last?.kind === "msg" && last.role === "assistant" && last.content) return "回答中";
  return "思考中";
}

/** 把 tokens 数压成易读的 1.2k / 34.5k 形式 */
export function fmtTokens(n: number): string {
  if (n < 1000) return `${n}`;
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`;
  return `${(n / 1_000_000).toFixed(2)}M`;
}

/** 占比 0~100 映射到颜色：占得越满，色相越偏红、明度越深 */
export function ringColor(pct: number): string {
  const hue = Math.round(140 - (140 * pct) / 100); // 绿(140) → 红(0)
  const light = Math.round(66 - (30 * pct) / 100); // 浅(66%) → 深(36%)
  return `hsl(${hue}, 72%, ${light}%)`;
}
