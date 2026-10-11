import { AgentSession, MemoryLogStore } from "@sage/core";
import {
  createFaux,
  FAUX_MODEL,
  FAUX_SYSTEM,
  recordViews,
  stopScript,
  toolScript,
} from "@sage/core/testing";
import { expect, it } from "vitest";
import { contextCompiler } from "../../src/compiler/compiler.ts";
import { rebuildFromLog } from "../../src/compiler/rebuild.ts";

function echoTool() {
  return {
    def: {
      name: "echo",
      description: "Echo text",
      inputSchema: { type: "object" },
    },
    isReadOnly: true,
    isDestructive: false,
    isOpenWorld: false,
    accesses() {
      return [];
    },
    async execute() {
      return { content: "ok", isError: false };
    },
  };
}

it("rebuilds each turn's head and messages from the log, including content addressed tails", async () => {
  const store = new MemoryLogStore();
  const sessionId = "rebuild";
  const { adapter, requests } = createFaux([
    toolScript("c1", "echo"),
    stopScript("done"),
  ]);
  const recorder = recordViews(
    contextCompiler({
      budget: 10000,
      claims: [
        (ctx) => [
          {
            id: "persona",
            priority: 90,
            placement: "epoch-head",
            tiers: { full: ctx.turn === 1 ? "careful" : "CHANGED" },
          },
          {
            id: "hint",
            priority: 40,
            placement: "tail",
            tiers: { full: `hint-${ctx.turn}` },
          },
          {
            id: "ledger",
            priority: 30,
            placement: "tail-if-changed",
            tiers: { full: "alpha" },
          },
        ],
      ],
    }),
  );
  const session = new AgentSession({
    store,
    sessionId,
    adapter,
    model: FAUX_MODEL,
    system: FAUX_SYSTEM,
    tools: [echoTool()],
    viewProvider: recorder,
  });

  await session.run("hello");
  const path = store.path(sessionId);
  const views = recorder.views;
  expect(views).toHaveLength(2);
  expect(views[0]?.messages[0]).toEqual(views[1]?.messages[0]);
  expect(JSON.stringify(views[1]?.messages[0])).not.toContain("CHANGED");

  const headers = path.filter((entry) => entry.kind === "request_header");
  expect(headers[1]?.payload).not.toHaveProperty("system");
  expect(headers[1]?.payload).not.toHaveProperty("tools");

  const manifests = path.filter((entry) => entry.kind === "manifest");
  const ledger = (index: number) => {
    const details = manifests[index]?.details?.claimContent as
      | Record<string, string>
      | undefined;
    return details?.ledger;
  };
  expect(ledger(0)).toContain("alpha");
  expect(ledger(1)).toBeUndefined();

  for (let turn = 1; turn <= requests.length; turn += 1) {
    const rebuilt = rebuildFromLog(path, turn);
    const sent = requests[turn - 1];
    if (!sent) throw new Error(`missing request ${turn}`);
    expect(rebuilt.head).toEqual(sent.head);
    expect(rebuilt.messages).toEqual(sent.messages);
  }
});
