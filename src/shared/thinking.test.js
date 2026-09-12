import { describe, it, expect } from "vitest";
import { thinkingControl, thinkingFor, BUDGETS, EFFORTS } from "./thinking.js";

/**
 * The control follows the model, the way opencode's variants do. Before this
 * every model got Anthropic's budget dial, which only Anthropic's transport
 * read - "Careful" on Minimax changed nothing and said otherwise.
 */
describe("which dial a model gets", () => {
  it("gives Claude on Anthropic's own protocol a thinking budget", () => {
    expect(thinkingControl("claude-sonnet-4-5", "anthropic")).toEqual({ kind: "budget", options: BUDGETS });
  });

  it("gives OpenAI's reasoning families an effort level", () => {
    for (const id of ["gpt-5", "openai/gpt-5.2-codex", "o3-mini", "o4-mini", "openai/gpt-oss-120b", "grok-4"]) {
      expect(thinkingControl(id, "openai").kind, id).toBe("effort");
    }
    expect(thinkingControl("gpt-5", "openai").options).toBe(EFFORTS);
  });

  it("does not mistake gpt-4o for a reasoning model", () => {
    expect(thinkingControl("gpt-4o", "openai").kind).toBe("none");
    expect(thinkingControl("gpt-4.1", "openai").kind).toBe("none");
  });

  it("says so for a model that decides for itself", () => {
    for (const id of ["MiniMax-M3", "Qwen/Qwen3.8-27B-TEE", "deepseek-ai/DeepSeek-V4-Flash", "glm-4.6", "kimi-k2"]) {
      const control = thinkingControl(id, "openai");
      expect(control.kind, id).toBe("none");
      expect(control.options[0].label).toBe("Not adjustable");
    }
  });

  it("offers Claude through a gateway an effort level, since that is what the gateway takes", () => {
    expect(thinkingControl("anthropic/claude-sonnet-4-5", "openai").kind).toBe("effort");
  });
});

describe("what gets sent", () => {
  it("sends the budget to Claude and nothing else", () => {
    expect(thinkingFor({ thinkingBudget: 8192, reasoningEffort: "high" }, "claude-opus-4", "anthropic")).toEqual({
      thinkingBudget: 8192,
      reasoningEffort: null,
    });
  });

  it("sends the effort to a reasoning model and no budget", () => {
    expect(thinkingFor({ thinkingBudget: 8192, reasoningEffort: "high" }, "gpt-5", "openai")).toEqual({
      thinkingBudget: 0,
      reasoningEffort: "high",
    });
  });

  it("sends neither to a model with no dial, whatever the agent has on file", () => {
    expect(thinkingFor({ thinkingBudget: 24576, reasoningEffort: "high" }, "MiniMax-M3", "openai")).toEqual({
      thinkingBudget: 0,
      reasoningEffort: null,
    });
  });

  it("treats the default effort, and an unknown one, as leaving the field out", () => {
    expect(thinkingFor({ reasoningEffort: "default" }, "gpt-5", "openai").reasoningEffort).toBeNull();
    expect(thinkingFor({ reasoningEffort: "ultra" }, "gpt-5", "openai").reasoningEffort).toBeNull();
    expect(thinkingFor({}, "gpt-5", "openai").reasoningEffort).toBeNull();
  });
});
