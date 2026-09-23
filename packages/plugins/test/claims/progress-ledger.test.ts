import { stopScript, toolScript } from "@sage/core/testing";
import { expect, it } from "vitest";
import { progressLedger } from "../../src/claims/index.ts";
import { contextCompiler } from "../../src/compiler/compiler.ts";
import { contextOf, runSession } from "./session.ts";

it("folds update_ledger inputs and does not validate or drive", async () => {
  const { tool, provider } = progressLedger();
  expect(tool.def.name).toBe("update_ledger");
  expect(tool.isReadOnly).toBe(true);
  expect(tool.accesses({ goal: "x" })).toEqual([]);
  expect(tool.def.inputSchema).toMatchObject({
    type: "object",
    properties: {
      goal: { type: "string" },
      plan: { type: "array", items: { type: "string" } },
      decisions: { type: "array", items: { type: "string" } },
    },
  });
  expect(
    await tool.execute(
      { plan: "not-an-array" },
      { sessionId: "s", toolUseId: "t" },
    ),
  ).toEqual({ content: "Ledger updated.", isError: false });

  const idle = await runSession({
    scripts: [stopScript("ok")],
    tools: [tool],
  });
  expect(await provider(contextOf(idle.store, idle.sessionId))).toEqual([]);

  const run = await runSession({
    scripts: [
      toolScript(
        "u1",
        "update_ledger",
        JSON.stringify({
          goal: "old",
          plan: ["a", "b"],
          decisions: ["keep"],
        }),
      ),
      toolScript(
        "u2",
        "update_ledger",
        JSON.stringify({ goal: "new", plan: ["c"] }),
      ),
      stopScript("ok"),
    ],
    tools: [tool],
  });
  const claims = await provider(contextOf(run.store, run.sessionId));
  expect(claims).toEqual([
    {
      id: "progress-ledger",
      priority: 100,
      pinned: true,
      placement: "tail-if-changed",
      tiers: {
        full: "Goal: new\nPlan:\n- c\nDecisions:\n- keep",
      },
    },
  ]);
});

it("keeps the same ledger hash for three turns and stores the body only once", async () => {
  const { tool, provider } = progressLedger();
  const compiler = contextCompiler({ budget: 40000, claims: [provider] });
  const { store, sessionId } = await runSession({
    scripts: [
      toolScript(
        "u1",
        "update_ledger",
        JSON.stringify({
          goal: "ship",
          plan: ["cut"],
          decisions: ["claims"],
        }),
      ),
      toolScript("r1", "read", JSON.stringify({ path: "a.ts" })),
      toolScript("r2", "read", JSON.stringify({ path: "b.ts" })),
      stopScript("done"),
    ],
    tools: [tool],
    viewProvider: compiler,
  });

  const manifests = store
    .path(sessionId)
    .filter((entry) => entry.kind === "manifest");
  expect(manifests).toHaveLength(4);

  const ledgerOf = (index: number) => {
    const payload = manifests[index]?.payload as {
      claims?: { id: string; contentHash?: string }[];
    };
    return payload.claims?.find((claim) => claim.id === "progress-ledger");
  };
  expect(ledgerOf(0)).toBeUndefined();
  const hashes = [1, 2, 3].map((index) => ledgerOf(index)?.contentHash);
  expect(hashes[0]).toBeTruthy();
  expect(hashes[1]).toBe(hashes[0]);
  expect(hashes[2]).toBe(hashes[0]);

  const bodyOf = (index: number) => {
    const details = manifests[index]?.details as
      | { claimContent?: Record<string, string> }
      | undefined;
    return details?.claimContent?.["progress-ledger"];
  };
  expect(bodyOf(1)).toContain("Goal: ship");
  expect(bodyOf(1)).toContain("Plan:\n- cut");
  expect(bodyOf(1)).toContain("Decisions:\n- claims");
  expect(bodyOf(2)).toBeUndefined();
  expect(bodyOf(3)).toBeUndefined();
});
