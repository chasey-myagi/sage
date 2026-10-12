import { describe, expect, it } from "vitest";
import { synthesizeToolResult, TOOL_OUTCOME_UNKNOWN } from "../../src/index.ts";

describe("dangling tool results", () => {
  it("returns the outcome-unknown model-facing text and does not scan history", () => {
    const unknown = synthesizeToolResult("call_2");

    expect(unknown).toEqual({
      type: "tool_result",
      toolUseId: "call_2",
      content: TOOL_OUTCOME_UNKNOWN,
      isError: true,
    });

    expect(TOOL_OUTCOME_UNKNOWN).toBe(
      "This tool call was started, but its result was not recorded. Check the external state it may have changed before you decide what to do. Do not retry blindly.",
    );
    expect(TOOL_OUTCOME_UNKNOWN).toMatch(/was started/i);
    expect(TOOL_OUTCOME_UNKNOWN).toMatch(/external state/i);
    expect(TOOL_OUTCOME_UNKNOWN).toMatch(/do not retry blindly/i);
  });
});
