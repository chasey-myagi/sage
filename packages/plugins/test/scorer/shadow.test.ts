import { MemoryLogStore, type ViewContext } from "@sage/core";
import type { Request } from "@sage/provider/contract";
import { expect, it } from "vitest";
import { contextCompiler } from "../../src/compiler/compiler.ts";
import type { Scorer } from "../../src/scorer/types.ts";

const MODEL = { provider: "anthropic", model: "claude-test" };
const HEAD = { system: "You are sage.", tools: [] as const };

type ShadowDecision = { id: string; tier: string; reason?: string };
type ShadowPayload = {
  scorer: string;
  decisions: ShadowDecision[];
  fallback?: string;
};

function context(store: MemoryLogStore, turn: number): ViewContext {
  return {
    sessionId: "s",
    turn,
    path: store.path("s"),
    model: MODEL,
    prepared: {
      model: MODEL,
      contextWindow: 1_000_000,
      pricing: null,
      adapterDefaults: {},
    },
    head: { system: HEAD.system, tools: [...HEAD.tools] },
    tools: [],
  };
}

function appendTool(store: MemoryLogStore, id: string, output: string): void {
  store.append("s", {
    kind: "message",
    payload: {
      role: "assistant",
      content: [
        {
          type: "tool_use",
          id,
          name: "read",
          input: { path: `${id}.ts` },
          rawInput: JSON.stringify({ path: `${id}.ts` }),
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
          toolUseId: id,
          content: output,
          isError: false,
        },
      ],
    },
  });
}

function seed(store: MemoryLogStore): void {
  store.append("s", {
    kind: "message",
    payload: {
      role: "user",
      content: [{ type: "text", text: "find the bug" }],
    },
  });
}

function toolIds(chunks: readonly { id: string }[]): string[] {
  return chunks
    .filter((chunk) => chunk.id.includes("#"))
    .map((chunk) => chunk.id);
}

function freshToolIds(
  chunks: readonly { id: string }[],
  previous: ReadonlySet<string>,
): string[] {
  return toolIds(chunks)
    .filter((id) => !previous.has(id))
    .sort();
}

async function compileTurns(shadowScorer?: Scorer): Promise<{
  views: Awaited<ReturnType<ReturnType<typeof contextCompiler>["view"]>>[];
  calls: { ids: string[]; request: Request | undefined }[];
}> {
  const calls: { ids: string[]; request: Request | undefined }[] = [];
  const wrapped: Scorer | undefined = shadowScorer
    ? {
        name: shadowScorer.name,
        score(input) {
          calls.push({
            ids: input.candidates.map((chunk) => chunk.id),
            request: input.prevRequest,
          });
          return shadowScorer.score(input);
        },
      }
    : undefined;
  const compiler = contextCompiler({
    budget: 100_000,
    claims: [],
    keepRecent: 6,
    ...(wrapped ? { shadowScorer: wrapped } : {}),
  });
  const store = new MemoryLogStore();
  seed(store);
  const views = [];
  for (let turn = 1; turn <= 5; turn += 1) {
    appendTool(store, `t${turn}`, `BODY-${turn}`);
    const view = await compiler.view(context(store, turn));
    views.push(view);
    store.append("s", { kind: "manifest", payload: view.manifest });
  }
  return { views, calls };
}

function shadowOf(manifest: { shadow?: unknown }): ShadowPayload {
  const shadow = manifest.shadow as ShadowPayload | undefined;
  if (!shadow) throw new Error("missing manifest.shadow");
  return shadow;
}

it("scores only new tool chunks and leaves tiers, budget, and freeze unchanged", async () => {
  const scorer: Scorer = {
    name: "fake",
    async score(input) {
      return {
        decisions: input.candidates.map((chunk) => ({
          id: chunk.id,
          tier: "none" as const,
          reason: "shadow",
        })),
      };
    },
  };
  const plain = await compileTurns();
  const shadowed = await compileTurns(scorer);

  expect(shadowed.calls).toHaveLength(5);
  expect(shadowed.views).toHaveLength(5);
  const seen = new Set<string>();
  for (let index = 0; index < 5; index += 1) {
    const plainView = plain.views[index];
    const shadowView = shadowed.views[index];
    if (!plainView || !shadowView) throw new Error(`missing turn ${index + 1}`);
    const fresh = freshToolIds(plainView.manifest.chunks, seen);
    for (const id of toolIds(plainView.manifest.chunks)) seen.add(id);

    const call = shadowed.calls[index];
    expect(call?.ids.slice().sort()).toEqual(fresh);
    const shadow = shadowOf(shadowView.manifest);
    expect(shadow.scorer).toBe("fake");
    expect(
      shadow.decisions
        .map((decision) => ({
          id: decision.id,
          tier: decision.tier,
          reason: decision.reason,
        }))
        .sort((left, right) => left.id.localeCompare(right.id)),
    ).toEqual(fresh.map((id) => ({ id, tier: "none", reason: "shadow" })));
    expect(JSON.stringify(shadowView.manifest.chunks)).toBe(
      JSON.stringify(plainView.manifest.chunks),
    );
    expect(JSON.stringify(shadowView.manifest.budget)).toBe(
      JSON.stringify(plainView.manifest.budget),
    );
    expect(JSON.stringify(shadowView.messages)).toBe(
      JSON.stringify(plainView.messages),
    );
    expect(shadowView.epoch).toBe(plainView.epoch);
    expect(call?.request?.messages).toEqual(shadowView.messages);
    expect(call?.request?.head).toEqual(shadowView.head);
    expect(call?.request?.cacheBreakpoints).toEqual(
      shadowView.cacheBreakpoints,
    );
  }
});

it("records shadow.fallback when the scorer throws and still compiles", async () => {
  const scorer: Scorer = {
    name: "fake",
    async score() {
      throw new Error("boom");
    },
  };
  const plain = await compileTurns();
  const shadowed = await compileTurns(scorer);
  const shadowView = shadowed.views[0];
  const plainView = plain.views[0];
  if (!shadowView || !plainView) throw new Error("missing turn 1");
  const shadow = shadowOf(shadowView.manifest);
  expect(shadow.fallback).toEqual(expect.any(String));
  expect(shadow.fallback?.length).toBeGreaterThan(0);
  expect(shadow.decisions).toEqual([]);
  expect(JSON.stringify(shadowView.manifest.chunks)).toBe(
    JSON.stringify(plainView.manifest.chunks),
  );
  expect(JSON.stringify(shadowView.manifest.budget)).toBe(
    JSON.stringify(plainView.manifest.budget),
  );
  expect(JSON.stringify(shadowView.messages)).toBe(
    JSON.stringify(plainView.messages),
  );
});
