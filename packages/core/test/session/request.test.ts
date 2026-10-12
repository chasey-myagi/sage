import { describe, expect, it } from "vitest";
import { runHealth } from "../../src/health.ts";
import {
  AgentSession,
  MemoryLogStore,
  passthroughView,
} from "../../src/index.ts";
import {
  createFaux,
  fauxAdapter,
  FAUX_MODEL as MODEL,
  FAUX_SYSTEM as SYSTEM,
  stopScript,
  toolScript,
  FAUX_USAGE as USAGE,
} from "../../src/testing/index.ts";
import { canonicalJson, sha256 } from "../support/canonical.ts";

function tool(name = "echo") {
  return {
    def: { name, description: name, inputSchema: { type: "object" } },
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

describe("the request envelope is logged and the request rebuilds from the Log", () => {
  it("logs the envelope and rebuilds the exact head and messages from the path", async () => {
    const store = new MemoryLogStore();
    const sessionId = "s3";
    const { adapter, requests, prepared } = createFaux(
      [stopScript("hello back")],
      { temperature: 0, maxTokens: 64 },
    );
    const session = new AgentSession({
      store,
      sessionId,
      adapters: { anthropic: adapter },
      model: MODEL,
      system: SYSTEM,
      tools: [tool()],
      sampling: { temperature: 0.2 },
    });

    const summary = await session.run("hello");
    expect(summary.outcome).toBe("done");
    expect(summary.turns).toBe(1);
    expect(summary.usage).toMatchObject({ output: USAGE.output });

    const path = store.path(sessionId);
    expect(path.map((entry) => entry.kind)).toEqual([
      "message",
      "request_header",
      "manifest",
      "message",
      "terminal",
    ]);
    expect(path[0]?.payload).toMatchObject({ role: "user" });
    expect(path[2]?.payload).toMatchObject({
      epoch: 0,
      turn: 1,
      claims: [],
      anomalies: [],
    });
    const manifest = path[2]?.payload as { chunks: { reason: string }[] };
    expect(manifest.chunks.length).toBeGreaterThan(0);
    for (const chunk of manifest.chunks) {
      expect(chunk.reason).toBe("passthrough");
    }
    expect(path[3]?.payload).toMatchObject({ role: "assistant" });
    expect(path[4]?.payload).toMatchObject({ outcome: "done" });
    expect(path[3]?.details).toEqual(
      expect.objectContaining({
        responseModel: expect.any(String),
        responseId: expect.any(String),
        api: "anthropic-messages",
        usage: expect.objectContaining({ output: USAGE.output }),
        finish: expect.objectContaining({ type: "finish", kind: "stop" }),
      }),
    );

    const header = path[1]?.payload as {
      systemPromptHash: string;
      toolsHash: string;
      sampling: { temperature?: number; maxTokens?: number };
      adapterDefaults: { temperature?: number; maxTokens?: number };
      system?: string;
      tools?: unknown;
    };
    const received = requests[0];
    if (!received) throw new Error("missing request");
    expect(header.systemPromptHash).toBe(
      sha256(canonicalJson(received.head.system)),
    );
    expect(header.toolsHash).toBe(sha256(canonicalJson(received.head.tools)));
    expect(header.system).toBe(SYSTEM);
    expect(header.tools).toEqual(received.head.tools);
    expect(header.sampling).toEqual({ temperature: 0.2, maxTokens: 64 });
    expect(received.sampling).toEqual({ temperature: 0.2, maxTokens: 64 });
    expect(header.adapterDefaults).toEqual(prepared.adapterDefaults);

    const prefix = path.slice(0, 1);
    const view = await passthroughView.view({
      sessionId,
      turn: 1,
      path: prefix,
      model: MODEL,
      prepared,
      head: {
        system: header.system ?? "",
        tools: header.tools as typeof received.head.tools,
      },
    });
    expect(canonicalJson({ head: view.head, messages: view.messages })).toBe(
      canonicalJson({ head: received.head, messages: received.messages }),
    );

    const health = runHealth(path);
    expect(summary.health).toEqual(health);
    expect(path.at(-1)?.payload).toMatchObject({ outcome: "done", health });
    expect(health.turns).toBe(summary.turns);
    expect(health.tokens).toEqual({
      input: USAGE.inputOther,
      cacheRead: USAGE.inputCacheRead,
      cacheCreation: USAGE.inputCacheCreation,
      output: USAGE.output,
      total:
        USAGE.inputOther +
        USAGE.inputCacheRead +
        USAGE.inputCacheCreation +
        USAGE.output,
    });
    expect(health.cacheHitRatio).toBe(
      USAGE.inputCacheRead /
        (USAGE.inputCacheRead + USAGE.inputOther + USAGE.inputCacheCreation),
    );
  });
});

describe("request headers store system and tools text only when it changes", () => {
  it("records the prepared call's context window and pricing on request_header", async () => {
    const store = new MemoryLogStore();
    const pricing = {
      inputPerMTok: 2,
      outputPerMTok: 10,
      cacheWritePerMTok: 2.5,
      cacheReadPerMTok: 0.2,
      source: "https://platform.claude.com/docs/en/about-claude/pricing",
    };
    const adapter = fauxAdapter([stopScript("hi")], {
      prepared: { contextWindow: 200_000, pricing },
    });
    const session = new AgentSession({
      store,
      sessionId: "priced",
      adapters: { anthropic: adapter },
      model: MODEL,
      system: SYSTEM,
      tools: [],
    });
    await session.run("hello");
    const header = store
      .path("priced")
      .find((entry) => entry.kind === "request_header");
    expect(header?.payload).toMatchObject({ contextWindow: 200_000, pricing });
  });

  it("omits system and tools text when the next head matches", async () => {
    const store = new MemoryLogStore();
    const sessionId = "s4";
    const { adapter } = createFaux([
      toolScript("c1", "echo"),
      stopScript("done"),
    ]);
    const session = new AgentSession({
      store,
      sessionId,
      adapters: { anthropic: adapter },
      model: MODEL,
      system: SYSTEM,
      tools: [tool()],
    });

    await session.run("go");
    const headers = store
      .path(sessionId)
      .filter((entry) => entry.kind === "request_header")
      .map((entry) => entry.payload as Record<string, unknown>);
    expect(headers).toHaveLength(2);
    expect(headers[0]).toHaveProperty("system", SYSTEM);
    expect(headers[0]).toHaveProperty("tools");
    expect(headers[1]).not.toHaveProperty("system");
    expect(headers[1]).not.toHaveProperty("tools");
    expect(headers[1]?.systemPromptHash).toBe(headers[0]?.systemPromptHash);
    expect(headers[1]?.toolsHash).toBe(headers[0]?.toolsHash);
  });

  it("stores full tools text again after the head changes in a new epoch", async () => {
    const store = new MemoryLogStore();
    const sessionId = "s4b";
    const extra = {
      name: "extra",
      description: "extra",
      inputSchema: { type: "object" },
    };
    let turn = 0;
    const { adapter } = createFaux([
      toolScript("c1", "echo"),
      stopScript("done"),
    ]);
    const session = new AgentSession({
      store,
      sessionId,
      adapters: { anthropic: adapter },
      model: MODEL,
      system: SYSTEM,
      tools: [tool()],
      viewProvider: {
        async view(ctx) {
          turn += 1;
          const base = await passthroughView.view(ctx);
          if (turn < 2) return base;
          store.append(sessionId, {
            kind: "view_epoch",
            payload: { epoch: 2, reason: "add tool" },
          });
          return {
            ...base,
            epoch: 2,
            manifest: { ...base.manifest, epoch: 2 },
            head: {
              system: ctx.head.system,
              tools: [...ctx.head.tools, extra],
            },
          };
        },
      },
    });

    await session.run("go");
    const headers = store
      .path(sessionId)
      .filter((entry) => entry.kind === "request_header")
      .map((entry) => entry.payload as Record<string, unknown>);
    expect(headers).toHaveLength(2);
    expect(headers[1]).not.toHaveProperty("system");
    expect(headers[1]?.tools).toEqual([
      { name: "echo", description: "echo", inputSchema: { type: "object" } },
      extra,
    ]);
  });
});
