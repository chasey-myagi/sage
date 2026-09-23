import type {
  PreparedCall,
  ProviderAdapter,
  Request,
} from "@sage/provider/contract";
import { describe, expect, it } from "vitest";
import {
  AgentSession,
  MemoryLogStore,
  passthroughView,
} from "../../src/index.ts";
import { FAUX_SYSTEM as SYSTEM } from "../../src/testing/index.ts";

const START = { provider: "anthropic", model: "claude-sonnet-5" };
const MINI = { provider: "openai", model: "gpt-5.4-mini" };
const HAIKU = { provider: "anthropic", model: "claude-haiku-4-5-20251001" };

const TOOL = {
  name: "echo",
  description: "echo",
  inputSchema: { type: "object" },
};

function harnessTool() {
  return {
    def: TOOL,
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

function scripted(
  api: ProviderAdapter["api"],
  scripts: Array<"stop" | "tool">,
) {
  const requests: { request: Request; prepared: PreparedCall }[] = [];
  const prepares: PreparedCall[] = [];
  let cursor = 0;
  const adapter: ProviderAdapter = {
    api,
    prepare(model) {
      const mini = model.model === "gpt-5.4-mini";
      const prepared: PreparedCall = {
        model: { provider: model.provider, model: model.model },
        contextWindow: mini ? 272_000 : 1_000_000,
        pricing: {
          inputPerMTok: mini ? 0.75 : 2,
          outputPerMTok: mini ? 4.5 : 10,
          cacheReadPerMTok: mini ? 0.075 : 0.2,
          cacheWritePerMTok: mini ? 0 : 2.5,
          source: "test",
        },
        adapterDefaults: { maxTokens: mini ? 111 : 222 },
      };
      prepares.push(structuredClone(prepared));
      return prepared;
    },
    async *stream(request, prepared) {
      requests.push({
        request: structuredClone(request),
        prepared: structuredClone(prepared),
      });
      const kind = scripts[cursor] ?? "stop";
      cursor += 1;
      if (kind === "tool") {
        yield {
          type: "block_start",
          index: 0,
          blockType: "tool_use",
          id: "c1",
          name: "echo",
        };
        yield { type: "tool_input_delta", index: 0, partialJson: "{}" };
        yield { type: "block_end", index: 0 };
        yield {
          type: "usage",
          usage: {
            inputOther: 1,
            inputCacheRead: 0,
            inputCacheCreation: 0,
            output: 1,
          },
        };
        yield { type: "finish", kind: "tool_use" };
        return;
      }
      yield { type: "block_start", index: 0, blockType: "text" };
      yield { type: "text_delta", index: 0, text: "ok" };
      yield { type: "block_end", index: 0 };
      yield {
        type: "usage",
        usage: {
          inputOther: 1,
          inputCacheRead: 0,
          inputCacheCreation: 0,
          output: 1,
        },
      };
      yield { type: "finish", kind: "stop" };
    },
  };
  return { adapter, requests, prepares };
}

describe("model_change", () => {
  it("switches adapter before view_epoch and recomputes prepare", async () => {
    const store = new MemoryLogStore();
    const sessionId = "switch";
    const anthropic = scripted("anthropic-messages", ["stop"]);
    const openai = scripted("openai-chat", ["stop"]);
    let seenHead = "";
    const session = new AgentSession({
      store,
      sessionId,
      adapter: anthropic.adapter,
      adapters: {
        anthropic: anthropic.adapter,
        openai: openai.adapter,
      },
      model: START,
      system: SYSTEM,
      tools: [harnessTool()],
      viewProvider: {
        async view(ctx) {
          const base = await passthroughView.view(ctx);
          return {
            ...base,
            epoch: base.epoch + 1,
            epochStart: { reason: "cost:1", model: MINI },
            manifest: {
              ...base.manifest,
              epoch: base.epoch + 1,
              epochDecision: {
                rebuild: true,
                oldTokens: 10,
                newTokens: 4,
                remainingTurns: 10,
                readRate: 0.1,
                writeRate: 1.25,
                savingsPerTurn: 0.6,
                extraCost: 0,
                paybackTurns: 0,
                reason: "immediate;route:cheaper",
              },
            },
          };
        },
      },
    });
    session.hooks.observe("view_compiled", ({ view }) => {
      seenHead = JSON.stringify(view.head);
    });

    const summary = await session.run("hello");
    expect(summary.outcome).toBe("done");
    expect(summary.modelChanges).toBe(1);

    const path = store.path(sessionId);
    expect(path.map((entry) => entry.kind)).toEqual([
      "message",
      "model_change",
      "view_epoch",
      "request_header",
      "manifest",
      "message",
      "terminal",
    ]);
    expect(path[1]?.payload).toEqual({
      from: START,
      to: MINI,
      reason: "route:cheaper",
    });
    expect(path[3]?.payload).toMatchObject({
      provider: "openai",
      model: "gpt-5.4-mini",
      adapterDefaults: { maxTokens: 111 },
    });
    expect(anthropic.requests).toHaveLength(0);
    expect(openai.requests).toHaveLength(1);
    expect(openai.requests[0]?.request.model).toEqual(MINI);
    expect(openai.requests[0]?.prepared.contextWindow).toBe(272_000);
    expect(openai.requests[0]?.prepared.pricing?.inputPerMTok).toBe(0.75);
    expect(openai.prepares.map((item) => item.model)).toEqual([MINI]);
    expect(anthropic.prepares.map((item) => item.contextWindow)).toEqual([
      1_000_000,
    ]);
    expect(JSON.stringify(openai.requests[0]?.request.head)).toBe(seenHead);
    expect(seenHead).toBe(JSON.stringify({ system: SYSTEM, tools: [TOOL] }));
  });

  it("does not write model_change when the route keeps the current model or abstains", async () => {
    const store = new MemoryLogStore();
    const sessionId = "same";
    const anthropic = scripted("anthropic-messages", ["tool", "stop"]);
    let turn = 0;
    const session = new AgentSession({
      store,
      sessionId,
      adapter: anthropic.adapter,
      adapters: { anthropic: anthropic.adapter },
      model: START,
      system: SYSTEM,
      tools: [harnessTool()],
      viewProvider: {
        async view(ctx) {
          turn += 1;
          const base = await passthroughView.view(ctx);
          if (turn === 1) {
            return {
              ...base,
              epoch: base.epoch + 1,
              epochStart: { reason: "cost:1", model: START },
              manifest: {
                ...base.manifest,
                epoch: base.epoch + 1,
                epochDecision: {
                  rebuild: true,
                  oldTokens: 1,
                  newTokens: 1,
                  remainingTurns: 1,
                  readRate: 0.1,
                  writeRate: 1.25,
                  savingsPerTurn: 0,
                  extraCost: 0,
                  paybackTurns: 0,
                  reason: "immediate;route:stay",
                },
              },
            };
          }
          return {
            ...base,
            epoch: base.epoch + 1,
            epochStart: { reason: "cost:1" },
            manifest: { ...base.manifest, epoch: base.epoch + 1 },
          };
        },
      },
    });

    const summary = await session.run("hello");
    expect(summary.modelChanges).toBe(0);
    expect(
      store.path(sessionId).some((entry) => entry.kind === "model_change"),
    ).toBe(false);
    expect(anthropic.requests).toHaveLength(2);
    expect(anthropic.requests[0]?.request.model).toEqual(START);
    expect(anthropic.prepares).toHaveLength(1);
  });

  it("counts every model_change in the run", async () => {
    const store = new MemoryLogStore();
    const sessionId = "twice";
    const anthropic = scripted("anthropic-messages", ["stop"]);
    const openai = scripted("openai-chat", ["tool"]);
    let turn = 0;
    const session = new AgentSession({
      store,
      sessionId,
      adapter: anthropic.adapter,
      adapters: {
        anthropic: anthropic.adapter,
        openai: openai.adapter,
      },
      model: START,
      system: SYSTEM,
      tools: [harnessTool()],
      viewProvider: {
        async view(ctx) {
          turn += 1;
          const base = await passthroughView.view(ctx);
          const model = turn === 1 ? MINI : HAIKU;
          const reason =
            turn === 1 ? "immediate;route:first" : "immediate;route:second";
          return {
            ...base,
            epoch: base.epoch + 1,
            epochStart: { reason: "cost:1", model },
            manifest: {
              ...base.manifest,
              epoch: base.epoch + 1,
              epochDecision: {
                rebuild: true,
                oldTokens: 1,
                newTokens: 1,
                remainingTurns: 1,
                readRate: 0.1,
                writeRate: 1.25,
                savingsPerTurn: 0,
                extraCost: 0,
                paybackTurns: 0,
                reason,
              },
            },
          };
        },
      },
    });

    const summary = await session.run("hello");
    expect(summary.outcome).toBe("done");
    expect(summary.modelChanges).toBe(2);
    const changes = store
      .path(sessionId)
      .filter((entry) => entry.kind === "model_change");
    expect(changes.map((entry) => entry.payload)).toEqual([
      { from: START, to: MINI, reason: "route:first" },
      { from: MINI, to: HAIKU, reason: "route:second" },
    ]);
    const heads = [...anthropic.requests, ...openai.requests].map((item) =>
      JSON.stringify(item.request.head),
    );
    expect(new Set(heads).size).toBe(1);
  });

  it("stops with a terminal error when the routed provider has no adapter", async () => {
    const store = new MemoryLogStore();
    const sessionId = "missing";
    const anthropic = scripted("anthropic-messages", ["stop"]);
    const session = new AgentSession({
      store,
      sessionId,
      adapter: anthropic.adapter,
      adapters: { anthropic: anthropic.adapter },
      model: START,
      system: SYSTEM,
      tools: [],
      viewProvider: {
        async view(ctx) {
          const base = await passthroughView.view(ctx);
          return {
            ...base,
            epoch: base.epoch + 1,
            epochStart: { reason: "cost:1", model: MINI },
            manifest: {
              ...base.manifest,
              epoch: base.epoch + 1,
              epochDecision: {
                rebuild: true,
                oldTokens: 1,
                newTokens: 1,
                remainingTurns: 1,
                readRate: 0.1,
                writeRate: 1.25,
                savingsPerTurn: 0,
                extraCost: 0,
                paybackTurns: 0,
                reason: "immediate;route:away",
              },
            },
          };
        },
      },
    });

    const summary = await session.run("hello");
    expect(summary.outcome).toBe("error");
    expect(summary.modelChanges).toBe(1);
    expect(summary.error).toMatch(/openai/);
    expect(anthropic.requests).toHaveLength(0);
    const path = store.path(sessionId);
    expect(path.map((entry) => entry.kind)).toEqual([
      "message",
      "model_change",
      "terminal",
    ]);
    expect(path.at(-1)?.payload).toMatchObject({ outcome: "error" });
  });
});
