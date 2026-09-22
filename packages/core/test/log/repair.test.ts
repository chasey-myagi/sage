import { describe, expect, it } from "vitest";
import { DANGLING, synthesizeToolResult } from "../../src/index.ts";

describe("dangling tool results", () => {
  it("returns the two model-facing texts and does not scan history", () => {
    const notStarted = synthesizeToolResult("call_1", "TOOL_NOT_STARTED");
    const unknown = synthesizeToolResult("call_2", "TOOL_OUTCOME_UNKNOWN");

    expect(notStarted).toEqual({
      type: "tool_result",
      toolUseId: "call_1",
      content: DANGLING.TOOL_NOT_STARTED,
      isError: true,
    });
    expect(unknown).toEqual({
      type: "tool_result",
      toolUseId: "call_2",
      content: DANGLING.TOOL_OUTCOME_UNKNOWN,
      isError: true,
    });

    expect(DANGLING.TOOL_NOT_STARTED).toMatch(/not started/i);
    expect(DANGLING.TOOL_NOT_STARTED).toMatch(/safe to retry/i);
    expect(DANGLING.TOOL_OUTCOME_UNKNOWN).toMatch(/was started/i);
    expect(DANGLING.TOOL_OUTCOME_UNKNOWN).toMatch(/external state/i);
    expect(DANGLING.TOOL_OUTCOME_UNKNOWN).toMatch(/do not retry blindly/i);
  });
});
