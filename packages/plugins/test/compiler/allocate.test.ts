import { expect, it } from "vitest";
import { allocate } from "../../src/compiler/budget.ts";
import type { Claim } from "../../src/compiler/claim.ts";

function countOf(table: Record<string, number>) {
  return (text: string) => {
    const tokens = table[text];
    if (tokens === undefined) {
      throw new Error(`no token count for ${JSON.stringify(text)}`);
    }
    return tokens;
  };
}

function claim(
  id: string,
  priority: number,
  tiers: Claim["tiers"],
  extra?: { pinned?: boolean },
): Claim {
  return {
    id,
    priority,
    placement: "tail",
    tiers,
    ...(extra?.pinned ? { pinned: true } : {}),
  };
}

it("downgrades low-priority claims from their highest tier until the set fits", () => {
  const count = countOf({
    "a-full": 3000,
    "a-line": 100,
    "b-full": 9000,
    "b-line": 100,
    "c-full": 6000,
    "c-line": 100,
  });
  const result = allocate(
    [
      claim("a", 90, { full: "a-full", line: "a-line" }),
      claim("b", 50, { full: "b-full", line: "b-line" }),
      claim("c", 10, { full: "c-full", line: "c-line" }),
    ],
    10000,
    count,
  );

  expect(result.overrun).toBe(0);
  expect(result.records.map((record) => record.reason)).toEqual([
    "fits",
    "budget:downgraded",
    "budget:downgraded",
  ]);
  expect(result.records).toEqual([
    { id: "a", tier: "full", reason: "fits", tokens: 3000 },
    { id: "b", tier: "line", reason: "budget:downgraded", tokens: 100 },
    { id: "c", tier: "line", reason: "budget:downgraded", tokens: 100 },
  ]);
  expect(result.chosen.get("a")).toBe("full");
  expect(result.chosen.get("b")).toBe("line");
  expect(result.chosen.get("c")).toBe("line");
});

it("keeps a pinned claim at full and drops every other claim to none when the pin overruns", () => {
  const count = countOf({
    pin: 20000,
    "x-full": 5000,
    "x-line": 100,
    "y-full": 4000,
    "y-summary": 800,
  });
  const result = allocate(
    [
      claim("pin", 100, { full: "pin" }, { pinned: true }),
      claim("x", 40, { full: "x-full", line: "x-line" }),
      claim("y", 30, { full: "y-full", summary: "y-summary" }),
    ],
    10000,
    count,
  );

  expect(result.records).toEqual([
    { id: "pin", tier: "full", reason: "pinned", tokens: 20000 },
    { id: "x", tier: "none", reason: "budget:downgraded", tokens: 0 },
    { id: "y", tier: "none", reason: "budget:downgraded", tokens: 0 },
  ]);
  expect(result.overrun).toBe(10000);
});

it("drops one available tier at a time and stops on the first tier that fits", () => {
  const count = countOf({ full: 100, summary: 40, line: 10 });
  const result = allocate(
    [claim("only", 1, { full: "full", summary: "summary", line: "line" })],
    50,
    count,
  );

  expect(result.records).toEqual([
    {
      id: "only",
      tier: "summary",
      reason: "budget:downgraded",
      tokens: 40,
    },
  ]);
  expect(result.overrun).toBe(0);
});
