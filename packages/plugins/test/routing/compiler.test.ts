import { MemoryLogStore, type ViewContext } from "@sage/core";
import { listModels } from "@sage/provider";
import { describe, expect, it } from "vitest";
import { contextCompiler } from "../../src/compiler/compiler.ts";
import {
  costTierPolicy,
  sensitivePolicy,
  withSensitiveFilter,
} from "../../src/routing/policy.ts";
import type { RouteInput, RoutePolicy } from "../../src/routing/types.ts";

const ENV = /(^|\/)\.env$|\/secrets\//;
const SONNET = listModels("anthropic").find(
  (entry) => entry.model === "claude-sonnet-5",
);
if (!SONNET) throw new Error("missing sonnet catalog entry");

const MODEL = { provider: "anthropic", model: "claude-sonnet-5" };

function readTool() {
  return {
    def: {
      name: "read",
      description: "read",
      inputSchema: { type: "object" },
    },
    isReadOnly: true,
    isDestructive: false,
    isOpenWorld: false,
    accesses(input: unknown) {
      if (typeof input !== "object" || input === null || !("path" in input)) {
        return [];
      }
      const path = (input as { path?: unknown }).path;
      return typeof path === "string"
        ? [{ key: path, mode: "read" as const }]
        : [];
    },
    async execute() {
      return { content: "", isError: false };
    },
  };
}

function seed(store: MemoryLogStore): void {
  store.append("s", {
    kind: "message",
    payload: {
      role: "user",
      content: [{ type: "text", text: "WIDE history" }],
    },
  });
  store.append("s", {
    kind: "message",
    payload: {
      role: "assistant",
      content: [
        {
          type: "tool_use",
          id: "toolu_env",
          name: "read",
          input: { path: ".env" },
          rawInput: '{"path":".env"}',
        },
      ],
    },
  });
  store.append("s", {
    kind: "message",
    payload: {
      role: "user",
      content: [
        {
          type: "tool_result",
          toolUseId: "toolu_env",
          content: "SECRET",
          isError: false,
        },
      ],
    },
  });
}

function context(store: MemoryLogStore): ViewContext {
  return {
    sessionId: "s",
    turn: 1,
    path: store.path("s"),
    model: MODEL,
    prepared: {
      model: MODEL,
      contextWindow: 100,
      pricing: SONNET.pricing,
      adapterDefaults: {},
    },
    head: { system: "You are sage.", tools: [] },
    tools: [readTool()],
  };
}

function wideCount(text: string): number {
  return text.includes("WIDE") ? 10_000 : 1;
}

describe("compiler route option", () => {
  it("does not call the policy when rebuild is false", async () => {
    const seen: RouteInput[] = [];
    const route: RoutePolicy = (input) => {
      seen.push(input);
      return {
        model: { provider: "openai", model: "gpt-5.4-mini" },
        reason: "nope",
      };
    };
    const compiler = contextCompiler({
      budget: 1_000_000,
      claims: [],
      route,
    });
    const store = new MemoryLogStore();
    store.append("s", {
      kind: "message",
      payload: { role: "user", content: [{ type: "text", text: "hello" }] },
    });
    const view = await compiler.view({
      ...context(store),
      prepared: {
        model: MODEL,
        contextWindow: 1_000_000,
        pricing: SONNET.pricing,
        adapterDefaults: {},
      },
    });
    expect(view.manifest.epochDecision?.rebuild).toBe(false);
    expect(seen).toHaveLength(0);
    expect(view.epochStart?.model).toBeUndefined();
  });

  it("writes the routed model onto epochStart and the reason onto the decision", async () => {
    const seen: RouteInput[] = [];
    const route: RoutePolicy = (input) => {
      seen.push(input);
      return {
        model: { provider: "openai", model: "gpt-5.4-mini" },
        reason: "cheaper",
      };
    };
    const compiler = contextCompiler({
      budget: 1_000_000,
      claims: [],
      count: wideCount,
      route,
    });
    const store = new MemoryLogStore();
    store.append("s", {
      kind: "message",
      payload: {
        role: "user",
        content: [{ type: "text", text: "WIDE" }],
      },
    });
    const view = await compiler.view(context(store));
    expect(view.manifest.epochDecision?.rebuild).toBe(true);
    expect(seen).toHaveLength(1);
    expect(seen[0]?.epoch).toBe(view.epoch);
    expect(seen[0]?.current).toEqual(MODEL);
    expect(seen[0]?.budget).toBe(1_000_000);
    expect(seen[0]?.candidates).toEqual([...listModels("anthropic")]);
    expect(view.epochStart?.model).toEqual({
      provider: "openai",
      model: "gpt-5.4-mini",
    });
    expect(view.epochStart?.reason).toBe("context-window-guard");
    expect(view.manifest.epochDecision?.reason).toContain("route:cheaper");
    expect(JSON.stringify(view.head)).toBe(
      JSON.stringify({ system: "You are sage.", tools: [] }),
    );
  });

  it("omits model_change material when the policy abstains or names the current model", async () => {
    const abstain = contextCompiler({
      budget: 1_000_000,
      claims: [],
      count: wideCount,
      route: () => undefined,
    });
    const store = new MemoryLogStore();
    store.append("s", {
      kind: "message",
      payload: { role: "user", content: [{ type: "text", text: "WIDE" }] },
    });
    const abstained = await abstain.view(context(store));
    expect(abstained.manifest.epochDecision?.rebuild).toBe(true);
    expect(abstained.epochStart?.model).toBeUndefined();
    expect(abstained.manifest.epochDecision?.reason).not.toContain("route:");

    const same = contextCompiler({
      budget: 1_000_000,
      claims: [],
      count: wideCount,
      route: () => ({ model: MODEL, reason: "stay" }),
    });
    const stayedStore = new MemoryLogStore();
    stayedStore.append("s", {
      kind: "message",
      payload: { role: "user", content: [{ type: "text", text: "WIDE" }] },
    });
    const stayedView = await same.view(context(stayedStore));
    expect(stayedView.epochStart?.model).toEqual(MODEL);
    expect(stayedView.manifest.epochDecision?.reason).toContain("route:stay");
  });

  it("passes new-view tiers and accesses into the sensitive and cost policies", async () => {
    const policy = withSensitiveFilter(
      costTierPolicy({
        ladder: [
          { provider: "openai", model: "gpt-5.4-mini" },
          { provider: "anthropic", model: "claude-haiku-4-5-20251001" },
          { provider: "openai", model: "gpt-5.4" },
        ],
        pick: "cheapest-allowed",
      }),
      sensitivePolicy({
        rules: [{ pattern: ENV, denyProviders: ["openai"] }],
      }),
    );
    const both = [...listModels("anthropic"), ...listModels("openai")];
    const seen: RouteInput[] = [];
    const route: RoutePolicy = (input) => {
      seen.push(input);
      return policy({ ...input, candidates: both });
    };
    const visible = contextCompiler({
      budget: 1_000_000,
      claims: [],
      count: wideCount,
      route,
    });
    const visibleStore = new MemoryLogStore();
    seed(visibleStore);
    const visibleView = await visible.view(context(visibleStore));
    const toolChunk = seen[0]?.viewChunks.find((item) =>
      (item.accesses ?? []).some((access) => access.key === ".env"),
    );
    expect(toolChunk?.tier).toBe("full");
    expect(toolChunk?.accesses).toEqual([{ key: ".env", mode: "read" }]);
    expect(visibleView.epochStart?.model).toEqual({
      provider: "anthropic",
      model: "claude-haiku-4-5-20251001",
    });
    expect(visibleView.manifest.epochDecision?.reason).toContain(
      "sensitive:.env",
    );
    expect(visibleView.epochStart?.model?.provider).not.toBe("openai");

    seen.length = 0;
    const hidden = contextCompiler({
      budget: 1,
      claims: [],
      count: wideCount,
      keepRecent: 0,
      route,
    });
    const hiddenStore = new MemoryLogStore();
    seed(hiddenStore);
    const hiddenView = await hidden.view(context(hiddenStore));
    const hiddenChunk = seen[0]?.viewChunks.find((item) =>
      (item.accesses ?? []).some((access) => access.key === ".env"),
    );
    expect(hiddenChunk?.tier).toBe("none");
    expect(hiddenView.epochStart?.model).toEqual({
      provider: "openai",
      model: "gpt-5.4-mini",
    });
    expect(hiddenView.manifest.epochDecision?.reason).not.toContain(
      "sensitive:",
    );
  });
});
