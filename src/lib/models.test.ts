import { describe, it, expect } from "vitest";
import { splitModelValue, MODEL_GROUPS } from "../lib/models";

describe("splitModelValue", () => {
  it("splits provider/model on first slash", () => {
    expect(splitModelValue("ark/doubao-seed-2.0-pro")).toEqual({
      provider: "ark",
      model: "doubao-seed-2.0-pro",
    });
  });

  it("keeps model intact when it contains slashes", () => {
    expect(splitModelValue("claude/claude-opus-4-8")).toEqual({
      provider: "claude",
      model: "claude-opus-4-8",
    });
  });

  it("every model value in MODEL_GROUPS splits into non-empty provider+model", () => {
    for (const group of MODEL_GROUPS) {
      for (const opt of group.options) {
        const { provider, model } = splitModelValue(opt.value);
        expect(provider.length).toBeGreaterThan(0);
        expect(model.length).toBeGreaterThan(0);
        // provider 必须是 registry 已知的三家之一
        expect(["ark", "xiaomi-mimo", "claude"]).toContain(provider);
      }
    }
  });
});
