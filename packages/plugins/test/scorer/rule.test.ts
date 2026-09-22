import { expect, it } from "vitest";
import type { Chunk } from "../../src/compiler/chunks.ts";
import { ruleScorer } from "../../src/scorer/rule.ts";

function chunk(id: string): Chunk {
  return {
    id,
    kind: "tool",
    messageIndex: 0,
    toolName: "read",
    input: { path: "a.ts" },
    tokens: 40,
    accesses: [],
  };
}

it("scores every candidate full and reports no usage", async () => {
  const result = await ruleScorer.score({
    candidates: [chunk("seq:1#a"), chunk("seq:3#b")],
    prevRequest: undefined,
    query: "find the bug",
  });

  expect(ruleScorer.name).toBe("rule");
  expect(result.usage).toBeUndefined();
  expect(result.fallback).toBeUndefined();
  expect(result.decisions).toEqual([
    { id: "seq:1#a", tier: "full" },
    { id: "seq:3#b", tier: "full" },
  ]);
});
