import {
  type CompiledView,
  MemoryLogStore,
  runHealth,
  type ViewContext,
} from "@sage/core";
import {
  FAUX_CONTEXT_WINDOW,
  type JsonFetchReply,
  type JsonFetchRequest,
  jsonFetch,
} from "@sage/core/testing";
import { describe, expect, it } from "vitest";
import { contextCompiler } from "../../src/compiler/compiler.ts";
import { jevScorer } from "../../src/scorer/jev.ts";
import { answerNouls, sentBody } from "../scorer/jev-fixtures.ts";

const MODEL = { provider: "anthropic", model: "claude-test" };
const HEAD = {
  system: "You are sage.",
  tools: [
    {
      name: "read",
      description: "read a file",
      inputSchema: { type: "object" },
    },
  ],
};
const BIG = Array.from({ length: 80 }, (_, i) => `const v${i} = ${i};`).join(
  "\n",
);
/** More tool chunks than jev keeps in flight, so later requests start after the failure. */
const CALL_IDS = ["t1", "t2", "t3", "t4", "t5", "t6"] as const;
/** The chunk whose jev request fails in every test. */
const FAILING = "seq:1#t1";
const NONE_BY_JEV = "scorer:jev:none:failure=0.00,locate=0.00,keep=0.00";

function seed(store: MemoryLogStore): void {
  const append = (payload: unknown) =>
    store.append("s", { kind: "message", payload });
  append({ role: "user", content: [{ type: "text", text: "find the bug" }] });
  for (const id of CALL_IDS) {
    append({
      role: "assistant",
      content: [
        {
          type: "tool_use",
          id,
          name: "grep",
          input: { pattern: id, path: `${id}.ts` },
          rawInput: JSON.stringify({ pattern: id, path: `${id}.ts` }),
        },
      ],
    });
    append({
      role: "user",
      content: [
        {
          type: "tool_result",
          toolUseId: id,
          content: `${id}.ts:1:export function ${id}() {}\n${BIG}`,
          isError: false,
        },
      ],
    });
  }
}

function context(
  store: MemoryLogStore,
  turn: number,
  window: number,
): ViewContext {
  return {
    sessionId: "s",
    depth: 0,
    turn,
    path: store.path("s"),
    model: MODEL,
    prepared: {
      model: MODEL,
      contextWindow: window,
      pricing: null,
      adapterDefaults: {},
    },
    head: {
      system: HEAD.system,
      tools: HEAD.tools.map((tool) => ({ ...tool })),
    },
    tools: [],
  };
}

function logManifest(store: MemoryLogStore, view: CompiledView): void {
  store.append("s", {
    kind: "request_header",
    payload: {
      provider: MODEL.provider,
      model: MODEL.model,
      systemPromptHash: "s",
      toolsHash: "t",
      sampling: {},
      adapterDefaults: {},
      contextWindow: FAUX_CONTEXT_WINDOW,
      pricing: null,
      system: HEAD.system,
      tools: HEAD.tools,
    },
  });
  store.append("s", { kind: "manifest", payload: view.manifest });
}

/** Every chunk answers all noes (tier none) unless `failing` says its request fails. */
function jevFetch(failing: (chunkId: string) => JsonFetchReply | undefined) {
  return jsonFetch(
    (request: JsonFetchRequest) =>
      failing(sentBody(request).state.chunk.id) ?? answerFor(request),
  );
}

function scorer(fake: ReturnType<typeof jsonFetch>) {
  return jevScorer({
    apiKey: "k",
    fetchImpl: fake.fetchImpl,
    sleepImpl: async () => {},
  });
}

/** The all-noes answer `jevFetch` gives a chunk that does not fail. */
const answerFor = answerNouls({});

/** A seeded store whose first view is logged, so the next tight view rebuilds. */
async function rebuildReady(): Promise<MemoryLogStore> {
  const store = new MemoryLogStore();
  seed(store);
  const first = await contextCompiler({
    budget: 100_000,
    claims: [],
    keepRecent: 1,
  }).view(context(store, 1, 1_000_000));
  logManifest(store, first);
  return store;
}

/** The rebuilding second view, scored by `jev` when given. */
function rebuildView(
  store: MemoryLogStore,
  jev?: ReturnType<typeof jevScorer>,
): Promise<CompiledView> {
  return contextCompiler({
    budget: 100_000,
    claims: [],
    keepRecent: 1,
    ...(jev ? { scorer: jev } : {}),
  }).view(context(store, 2, 1));
}

function chunk(view: CompiledView, id: string) {
  return view.manifest.chunks.find((row) => row.id === id);
}

describe("jev partial failure", () => {
  it("applies the answered chunks on entry and leaves the failed chunk full, counting it in the manifest and health", async () => {
    const store = new MemoryLogStore();
    seed(store);
    const fake = jevFetch((id) =>
      id === FAILING ? { status: 500, text: "no" } : undefined,
    );
    const unscored = await contextCompiler({
      budget: 100_000,
      claims: [],
    }).view(context(store, 1, 1_000_000));
    const view = await contextCompiler({
      budget: 100_000,
      claims: [],
      scorerApply: "entry",
      minScoreTokens: 0,
      scorer: scorer(fake),
    }).view(context(store, 1, 1_000_000));
    logManifest(store, view);

    expect(fake.requests()).toHaveLength(CALL_IDS.length);
    expect(chunk(unscored, FAILING)?.tier).toBe("full");
    expect(chunk(view, FAILING)).toEqual(chunk(unscored, FAILING));
    for (const [index, id] of CALL_IDS.entries()) {
      if (index === 0) continue;
      expect(chunk(view, `seq:${index * 2 + 1}#${id}`)).toMatchObject({
        tier: "none",
        reason: `scorer:entry:none;${NONE_BY_JEV}`,
      });
    }
    expect(view.manifest.entryScored).toMatchObject({
      calls: 1,
      none: 5,
      failedRequests: 1,
    });
    expect(runHealth(store.path("s")).entryScored.failedRequests).toBe(1);
  });

  it("applies the answered chunks on a rebuild after one request fails, without a fallback", async () => {
    const store = new MemoryLogStore();
    seed(store);
    const first = await contextCompiler({
      budget: 100_000,
      claims: [],
      keepRecent: 1,
    }).view(context(store, 1, 1_000_000));
    logManifest(store, first);

    const fake = jevFetch((id) =>
      id === FAILING ? { status: 500, text: "no" } : undefined,
    );
    const unscored = await contextCompiler({
      budget: 100_000,
      claims: [],
      keepRecent: 1,
    }).view(context(store, 2, 1));
    const second = await contextCompiler({
      budget: 100_000,
      claims: [],
      keepRecent: 1,
      scorer: scorer(fake),
    }).view(context(store, 2, 1));
    logManifest(store, second);

    expect(second.manifest.epochDecision?.rebuild).toBe(true);
    expect(second.manifest.epochDecision?.reason).not.toContain(
      "scorer:fallback",
    );
    expect(fake.requests()).toHaveLength(5);
    expect(chunk(unscored, FAILING)?.tier).toBe("full");
    expect(chunk(second, FAILING)).toEqual(chunk(unscored, FAILING));
    for (const id of ["seq:3#t2", "seq:5#t3", "seq:7#t4", "seq:9#t5"]) {
      expect(chunk(second, id)).toMatchObject({
        tier: "none",
        reason: NONE_BY_JEV,
      });
    }
    expect(second.manifest.scorer).toEqual({
      calls: 1,
      usage: {
        inputOther: 40,
        inputCacheRead: 0,
        inputCacheCreation: 0,
        output: 8,
      },
      fallback: false,
      failedRequests: 1,
    });
    const health = runHealth(store.path("s"));
    expect(health.scorerFailedRequests).toBe(1);
    expect(health.scorerFallbacks).toBe(0);
  });

  it("stops starting requests on a rebuild once one exhausts its 429 retries, while in-flight requests finish and keep their decisions", async () => {
    const store = await rebuildReady();
    let releaseInFlight: () => void = () => {};
    const inFlight = new Promise<void>((resolve) => {
      releaseInFlight = resolve;
    });
    let failingAttempts = 0;
    const fake = jsonFetch(async (request) => {
      const id = sentBody(request).state.chunk.id;
      if (id === FAILING) {
        failingAttempts += 1;
        // Release the other in-flight requests only after the scorer has
        // seen the last 429, so the breaker is open before they finish.
        if (failingAttempts === 4) setTimeout(releaseInFlight, 0);
        return { status: 429, text: "busy" };
      }
      await inFlight;
      return answerFor(request);
    });
    const unscored = await rebuildView(store);
    const second = await rebuildView(store, scorer(fake));
    logManifest(store, second);

    const sentIds = fake.requests().map((r) => sentBody(r).state.chunk.id);
    expect(sentIds.filter((id) => id === FAILING)).toHaveLength(4);
    expect(sentIds.filter((id) => id !== FAILING).sort()).toEqual([
      "seq:3#t2",
      "seq:5#t3",
      "seq:7#t4",
    ]);
    expect(second.manifest.epochDecision?.reason).not.toContain(
      "scorer:fallback",
    );
    for (const id of ["seq:3#t2", "seq:5#t3", "seq:7#t4"]) {
      expect(chunk(second, id)).toMatchObject({
        tier: "none",
        reason: NONE_BY_JEV,
      });
    }
    for (const id of [FAILING, "seq:9#t5"]) {
      expect(chunk(second, id)).toEqual(chunk(unscored, id));
    }
    expect(second.manifest.scorer).toEqual({
      calls: 1,
      usage: {
        inputOther: 30,
        inputCacheRead: 0,
        inputCacheCreation: 0,
        output: 6,
      },
      fallback: false,
      failedRequests: 2,
    });
    expect(runHealth(store.path("s")).scorerFailedRequests).toBe(2);
  });

  it("falls back with http:429 on a rebuild when every in-flight request exhausts its retries and the breaker stops the rest", async () => {
    const store = await rebuildReady();
    const fake = jevFetch(() => ({ status: 429, text: "busy" }));
    const second = await rebuildView(store, scorer(fake));
    logManifest(store, second);

    const sentIds = new Set(
      fake.requests().map((r) => sentBody(r).state.chunk.id),
    );
    expect([...sentIds].sort()).toEqual([
      "seq:1#t1",
      "seq:3#t2",
      "seq:5#t3",
      "seq:7#t4",
    ]);
    expect(fake.requests()).toHaveLength(16);
    expect(second.manifest.epochDecision?.reason).toContain(
      ";scorer:fallback:http:429",
    );
    expect(second.manifest.scorer).toMatchObject({ calls: 1, fallback: true });
    expect(second.manifest.scorer).not.toHaveProperty("failedRequests");
    const health = runHealth(store.path("s"));
    expect(health.scorerFallbacks).toBe(1);
    expect(health.scorerFailedRequests).toBe(0);
  });

  it("records the answered chunks in shadow and counts the failed request", async () => {
    const store = new MemoryLogStore();
    seed(store);
    const fake = jevFetch((id) =>
      id === FAILING ? { status: 200, text: "not-json" } : undefined,
    );
    const view = await contextCompiler({
      budget: 100_000,
      claims: [],
      shadowScorer: scorer(fake),
    }).view(context(store, 1, 1_000_000));
    logManifest(store, view);

    const shadow = view.manifest.shadow;
    expect(shadow?.fallback).toBeUndefined();
    expect(shadow?.failedRequests).toBe(1);
    expect(shadow?.decisions.map((decision) => decision.id)).toEqual([
      "seq:3#t2",
      "seq:5#t3",
      "seq:7#t4",
      "seq:9#t5",
      "seq:11#t6",
    ]);
    expect(runHealth(store.path("s")).shadow.failedRequests).toBe(1);
  });

  it("still falls back on a rebuild when every request fails, and counts no partial failure", async () => {
    const store = new MemoryLogStore();
    seed(store);
    const first = await contextCompiler({
      budget: 100_000,
      claims: [],
      keepRecent: 1,
    }).view(context(store, 1, 1_000_000));
    logManifest(store, first);

    const fake = jevFetch(() => ({ status: 500, text: "no" }));
    const second = await contextCompiler({
      budget: 100_000,
      claims: [],
      keepRecent: 1,
      scorer: scorer(fake),
    }).view(context(store, 2, 1));
    logManifest(store, second);

    expect(second.manifest.epochDecision?.reason).toContain(
      ";scorer:fallback:http:500",
    );
    expect(second.manifest.scorer).toMatchObject({ calls: 1, fallback: true });
    expect(second.manifest.scorer).not.toHaveProperty("failedRequests");
    for (const row of second.manifest.chunks) {
      expect(row.reason.startsWith("scorer:jev")).toBe(false);
    }
    const health = runHealth(store.path("s"));
    expect(health.scorerFallbacks).toBe(1);
    expect(health.scorerFailedRequests).toBe(0);
  });

  it("leaves every candidate full on entry when every request fails", async () => {
    const store = new MemoryLogStore();
    seed(store);
    const fake = jevFetch(() => ({ status: 500, text: "no" }));
    const view = await contextCompiler({
      budget: 100_000,
      claims: [],
      scorerApply: "entry",
      minScoreTokens: 0,
      scorer: scorer(fake),
    }).view(context(store, 1, 1_000_000));
    logManifest(store, view);

    expect(view.manifest.chunks.every((row) => row.tier === "full")).toBe(true);
    expect(view.manifest.entryScored).toEqual({
      calls: 1,
      none: 0,
      summary: 0,
      full: 0,
      summaryRejected: 0,
    });
    expect(runHealth(store.path("s")).entryScored.failedRequests).toBe(0);
  });
});
