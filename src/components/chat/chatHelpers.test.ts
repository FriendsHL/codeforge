import { describe, expect, it } from "vitest";
import type { ChatItem } from "../../stores/chatStore";
import { fmtTokens, isSubagentTool, ringColor, toBlocks, workingLabel } from "./chatHelpers";

const tool = (id: string, done = true): ChatItem =>
  ({ kind: "tool", id, name: "read_file", done } as ChatItem);
const msg = (role: "user" | "assistant", content: string): ChatItem =>
  ({ kind: "msg", role, content } as ChatItem);

describe("isSubagentTool", () => {
  it("识别带 -sN- 前缀的子 agent 工具", () => {
    expect(isSubagentTool(tool("run-s0-abc"))).toBe(true);
    expect(isSubagentTool(tool("run-abc"))).toBe(false);
    expect(isSubagentTool(msg("user", "hi"))).toBe(false);
  });
});

describe("toBlocks", () => {
  it("普通条目逐个成块", () => {
    const blocks = toBlocks([msg("user", "a"), msg("assistant", "b")]);
    expect(blocks).toHaveLength(2);
    expect(blocks.every((b) => b.type === "item")).toBe(true);
  });

  it("连续的子 agent 工具合并成一个折叠组", () => {
    const blocks = toBlocks([
      msg("user", "go"),
      tool("r-s0-1"),
      tool("r-s1-2"),
      msg("assistant", "done"),
    ]);
    expect(blocks).toHaveLength(3);
    expect(blocks[1].type).toBe("subagents");
    if (blocks[1].type === "subagents") expect(blocks[1].items).toHaveLength(2);
  });

  it("非连续的子 agent 工具不会跨普通条目合并", () => {
    const blocks = toBlocks([tool("r-s0-1"), msg("assistant", "x"), tool("r-s0-2")]);
    expect(blocks.filter((b) => b.type === "subagents")).toHaveLength(2);
  });
});

describe("workingLabel", () => {
  it("按最后条目推断状态", () => {
    expect(workingLabel([{ kind: "approval", decision: null } as unknown as ChatItem])).toBe("等待你的审批");
    expect(workingLabel([tool("t", false)])).toBe("执行工具中");
    expect(workingLabel([msg("assistant", "答")])).toBe("回答中");
    expect(workingLabel([msg("assistant", "")])).toBe("思考中");
    expect(workingLabel([])).toBe("思考中");
  });
});

describe("fmtTokens", () => {
  it("分档压缩", () => {
    expect(fmtTokens(0)).toBe("0");
    expect(fmtTokens(999)).toBe("999");
    expect(fmtTokens(1500)).toBe("1.5k");
    expect(fmtTokens(42000)).toBe("42k");
    expect(fmtTokens(1_500_000)).toBe("1.50M");
  });
});

describe("ringColor", () => {
  it("占比越高越偏红越深", () => {
    expect(ringColor(0)).toBe("hsl(140, 72%, 66%)");
    expect(ringColor(100)).toBe("hsl(0, 72%, 36%)");
    // 中间值色相在两端之间
    expect(ringColor(50)).toBe("hsl(70, 72%, 51%)");
  });
});
