import { describe, expect, it } from "vitest";
import {
  AgentSession,
  MemoryLogStore,
  passthroughView,
} from "../../src/index.ts";
import {
  createFaux,
  MODEL,
  SYSTEM,
  stopScript,
  toolScript,
} from "../support/faux.ts";

const HEAD = {
  role: "user" as const,
  content: [{ type: "text" as const, text: "EPOCH HEAD" }],
};

describe("scenario 4 view_epoch", () => {
  it("appends view_epoch before request_header when a wrapper advances the epoch", async () => {
    const store = new MemoryLogStore();
    const sessionId = "epoch-advance";
    const { adapter, requests } = createFaux([stopScript("ok")]);
    const session = new AgentSession({
      store,
      sessionId,
      adapter,
      model: MODEL,
      system: SYSTEM,
      tools: [],
      viewProvider: {
        async view(ctx) {
          const base = await passthroughView.view(ctx);
          return {
            ...base,
            epoch: base.epoch + 1,
            epochStart: { reason: "test:advance" },
            messages: [HEAD, ...base.messages],
            manifest: { ...base.manifest, epoch: base.epoch + 1 },
            details: { claimContent: { note: "kept" } },
          };
        },
      },
    });

    await session.run("hello");

    const path = store.path(sessionId);
    expect(path.map((entry) => entry.kind)).toEqual([
      "message",
      "view_epoch",
      "request_header",
      "manifest",
      "message",
      "terminal",
    ]);
    const epoch = path[1];
    expect(epoch?.payload).toEqual({ epoch: 1, reason: "test:advance" });
    expect(epoch?.details?.epochHead).toEqual(HEAD);
    expect(requests[0]?.messages[0]).toEqual(HEAD);
    expect(path[3]?.details).toEqual({ claimContent: { note: "kept" } });
  });

  it("does not append another view_epoch while the epoch stays put", async () => {
    const store = new MemoryLogStore();
    const sessionId = "epoch-hold";
    let turn = 0;
    const { adapter } = createFaux([
      toolScript("c1", "echo"),
      stopScript("done"),
    ]);
    const session = new AgentSession({
      store,
      sessionId,
      adapter,
      model: MODEL,
      system: SYSTEM,
      tools: [
        {
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
        },
      ],
      viewProvider: {
        async view(ctx) {
          turn += 1;
          const base = await passthroughView.view(ctx);
          if (turn > 1) {
            return {
              ...base,
              epoch: 1,
              manifest: { ...base.manifest, epoch: 1 },
            };
          }
          return {
            ...base,
            epoch: 1,
            epochStart: { reason: "test:advance" },
            messages: [HEAD, ...base.messages],
            manifest: { ...base.manifest, epoch: 1 },
          };
        },
      },
    });

    await session.run("go");
    const epochs = store
      .path(sessionId)
      .filter((entry) => entry.kind === "view_epoch");
    expect(epochs).toHaveLength(1);
    expect(epochs[0]?.payload).toEqual({ epoch: 1, reason: "test:advance" });
    const headerAt = store
      .path(sessionId)
      .findIndex((entry) => entry.kind === "request_header");
    expect(headerAt).toBeGreaterThan(0);
    expect(store.path(sessionId)[headerAt - 1]?.kind).toBe("view_epoch");
  });
});
