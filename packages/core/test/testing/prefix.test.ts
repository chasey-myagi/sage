import { expect, it } from "vitest";
import {
  AgentSession,
  MemoryLogStore,
  passthroughView,
} from "../../src/index.ts";
import {
  FAUX_UNIT_USAGE,
  fauxAdapter,
  renderLlmInput,
  stopScript,
  toolScript,
} from "../../src/testing/index.ts";

function echo() {
  return {
    def: {
      name: "echo",
      description: "echo",
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

it("reprints the whole prompt when a view stamps the date into system", async () => {
  const store = new MemoryLogStore();
  const adapter = fauxAdapter(
    [
      toolScript("c1", "echo", "{}", FAUX_UNIT_USAGE),
      stopScript("done", FAUX_UNIT_USAGE),
    ],
    {
      prepared: { adapterDefaults: { maxTokens: 16 } },
    },
  );
  const session = new AgentSession({
    store,
    sessionId: "date",
    adapters: { anthropic: adapter },
    model: { provider: "anthropic", model: "claude-test" },
    system: "base",
    tools: [echo()],
    viewProvider: {
      async view(ctx) {
        const base = await passthroughView.view(ctx);
        const day = ctx.turn === 1 ? "2026-09-22" : "2026-09-23";
        const epoch = ctx.turn === 1 ? base.epoch : 1;
        if (ctx.turn > 1) {
          store.append(ctx.sessionId, {
            kind: "view_epoch",
            payload: { epoch: 1, reason: "date rolled" },
          });
        }
        return {
          ...base,
          epoch,
          manifest: { ...base.manifest, epoch },
          head: { ...base.head, system: `today is ${day}` },
        };
      },
    },
  });

  await session.run("hi");
  const first = adapter.requests[0];
  const second = adapter.requests[1];
  if (!first || !second) throw new Error("expected two requests");
  expect(renderLlmInput(first)).toContain("today is 2026-09-22");
  expect(renderLlmInput(second, first)).not.toContain("<last>");
  expect(renderLlmInput(second, first)).toMatchInlineSnapshot(`
    "system: today is 2026-09-23
    tools: [
      {
        "description": "echo",
        "inputSchema": {
          "type": "object"
        },
        "name": "echo"
      }
    ]
    messages:
    [user] hi
    [assistant] tool_use echo {}
    [user] tool_result c1 ok"
  `);
});
