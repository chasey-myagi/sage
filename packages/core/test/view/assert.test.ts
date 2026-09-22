import { describe, expect, it } from "vitest";
import {
  assertHeadFrozen,
  assertPaired,
  HeadChangedMidEpochError,
  headHash,
  systemHash,
  toolsHash,
  UnpairedToolCallError,
} from "../../src/index.ts";
import { canonicalJson, sha256 } from "../support/canonical.ts";

const toolA = {
  name: "a",
  description: "tool a",
  inputSchema: { type: "object" },
};
const toolB = {
  name: "b",
  description: "tool b",
  inputSchema: { type: "object", properties: { q: { type: "string" } } },
};

const headA = { system: "sys", tools: [toolA] };
const headB = { system: "sys", tools: [toolA, toolB] };

function headerFor(head: { system: string; tools: unknown[] }) {
  return {
    provider: "anthropic",
    model: "claude-test",
    systemPromptHash: sha256(canonicalJson(head.system)),
    toolsHash: sha256(canonicalJson(head.tools)),
    sampling: {},
    adapterDefaults: {},
  };
}

function view(
  epoch: number,
  head: { system: string; tools: typeof headA.tools },
) {
  return {
    epoch,
    head,
    messages: [],
    cacheBreakpoints: ["head" as const],
    manifest: {
      epoch,
      turn: 1,
      claims: [],
      chunks: [],
      anomalies: [],
    },
  };
}

function toolUse(id: string) {
  return {
    type: "tool_use" as const,
    id,
    name: "a",
    input: {},
    rawInput: "{}",
  };
}

function toolResult(id: string) {
  return {
    type: "tool_result" as const,
    toolUseId: id,
    content: "ok",
    isError: false,
  };
}

function thrown(run: () => void): unknown {
  try {
    run();
  } catch (error) {
    return error;
  }
  return undefined;
}

describe("assertPaired", () => {
  it("throws when a tool_use has no tool_result", () => {
    const error = thrown(() =>
      assertPaired([{ role: "assistant", content: [toolUse("call_1")] }]),
    );
    expect(error).toBeInstanceOf(UnpairedToolCallError);
  });

  it("throws when a tool_result has no tool_use", () => {
    const error = thrown(() =>
      assertPaired([{ role: "user", content: [toolResult("orphan")] }]),
    );
    expect(error).toBeInstanceOf(UnpairedToolCallError);
  });

  it("accepts a matched pair and an empty transcript", () => {
    expect(() => assertPaired([])).not.toThrow();
    expect(() =>
      assertPaired([
        { role: "assistant", content: [toolUse("a"), toolUse("b")] },
        { role: "user", content: [toolResult("a"), toolResult("b")] },
      ]),
    ).not.toThrow();
  });
});

describe("head hashes", () => {
  it("hashes canonical JSON of the system string, the tools array, and the head", () => {
    expect(systemHash(headA.system)).toBe(sha256(canonicalJson(headA.system)));
    expect(toolsHash(headA.tools)).toBe(sha256(canonicalJson(headA.tools)));
    expect(headHash(headA)).toBe(sha256(canonicalJson(headA)));
    expect(toolsHash(headA.tools)).not.toBe(toolsHash(headB.tools));
  });
});

describe("assertHeadFrozen", () => {
  it("allows the first header and an unchanged head inside an epoch", () => {
    expect(() => assertHeadFrozen(view(0, headB), undefined, 0)).not.toThrow();
    expect(() =>
      assertHeadFrozen(view(0, headA), headerFor(headA), 0),
    ).not.toThrow();
  });

  it("throws when the head changes without a newer epoch", () => {
    const error = thrown(() =>
      assertHeadFrozen(view(0, headB), headerFor(headA), 0),
    );
    expect(error).toBeInstanceOf(HeadChangedMidEpochError);
    expect((error as Error).message).toMatch(/epoch/i);
  });

  it("allows a head change when the view epoch is newer than the frozen epoch", () => {
    expect(() =>
      assertHeadFrozen(view(2, headB), headerFor(headA), 0),
    ).not.toThrow();
  });
});
