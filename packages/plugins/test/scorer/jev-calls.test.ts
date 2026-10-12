import { MemoryLogStore, runHealth, type ViewContext } from "@sage/core";
import { FAUX_CONTEXT_WINDOW, jsonFetch } from "@sage/core/testing";
import type { Usage } from "@sage/provider/contract";
import { describe, expect, it } from "vitest";
import { progressLedger } from "../../src/claims/progress-ledger.ts";
import { contextCompiler } from "../../src/compiler/compiler.ts";
import { jevScorer } from "../../src/scorer/jev.ts";
import {
  answerNouls,
  chunkOf,
  type FixtureCall,
  requestWith,
  sentBody,
} from "./jev-fixtures.ts";

const BUILD: FixtureCall = {
  id: "t1",
  tool: "bash",
  input: { command: "pnpm build" },
  output: "[bash exit code 0]\nbuilt src/index.ts:1 in 2s",
};
const LINT: FixtureCall = {
  id: "t2",
  tool: "bash",
  input: { command: "pnpm lint" },
  output: "[bash exit code 0]\nchecked 40 files",
};

function input(calls: FixtureCall[] = [BUILD]) {
  return {
    candidates: calls.map((call, index) => chunkOf(call, index * 2 + 1)),
    prevRequest: requestWith(calls),
    query: "q",
  };
}

describe("http", () => {
  it("retries a 429 twice with exponential backoff and then succeeds", async () => {
    const ok = answerNouls({ "seq:1#t1": { reports_failure: 0.9 } });
    const fake = jsonFetch((request, index) =>
      index < 2 ? { status: 429, text: "busy" } : ok(request),
    );
    const sleeps: number[] = [];
    const result = await jevScorer({
      apiKey: "k",
      fetchImpl: fake.fetchImpl,
      sleepImpl: async (ms) => {
        sleeps.push(ms);
      },
    }).score(input());
    expect(fake.requests()).toHaveLength(3);
    expect(sleeps).toEqual([500, 1000]);
    expect(result.fallback).toBeUndefined();
    expect(result.decisions.map((decision) => decision.tier)).toEqual(["full"]);
  });

  it("retries 529 and stops with http:429 after three failed retries", async () => {
    const sleeps: number[] = [];
    const sleepImpl = async (ms: number) => {
      sleeps.push(ms);
    };
    const once = jsonFetch((request, index) =>
      index === 0
        ? { status: 529, text: "overloaded" }
        : answerNouls({})(request),
    );
    const recovered = await jevScorer({
      apiKey: "k",
      fetchImpl: once.fetchImpl,
      sleepImpl,
    }).score(input());
    expect(once.requests()).toHaveLength(2);
    expect(sleeps).toEqual([500]);
    expect(recovered.fallback).toBeUndefined();

    sleeps.length = 0;
    const busy = jsonFetch(() => ({ status: 429, text: "busy" }));
    const exhausted = await jevScorer({
      apiKey: "k",
      fetchImpl: busy.fetchImpl,
      sleepImpl,
    }).score(input());
    expect(busy.requests()).toHaveLength(4);
    expect(sleeps).toEqual([500, 1000, 2000]);
    expect(exhausted).toEqual({ decisions: [], fallback: "http:429" });
  });

  it("hands the caller's signal to the retry wait and does not fetch again once it aborts", async () => {
    const controller = new AbortController();
    const waits: (AbortSignal | undefined)[] = [];
    const fake = jsonFetch(() => ({ status: 429, text: "busy" }));
    const result = await jevScorer({
      apiKey: "k",
      fetchImpl: fake.fetchImpl,
      sleepImpl: async (_ms, signal) => {
        waits.push(signal);
        controller.abort();
      },
    }).score({ ...input(), signal: controller.signal });
    expect(waits).toHaveLength(1);
    expect(waits[0]?.aborted).toBe(true);
    expect(fake.requests()).toHaveLength(1);
    expect(result).toEqual({ decisions: [], fallback: "aborted" });
  });

  it("waits for retry-after when the response gives one, capped at 30 seconds", async () => {
    const ok = answerNouls({});
    const replies = [
      { status: 429, text: "busy", headers: { "retry-after": "2" } },
      { status: 429, text: "busy", headers: { "retry-after": "120" } },
      { status: 429, text: "busy" },
    ];
    const fake = jsonFetch((request, index) => replies[index] ?? ok(request));
    const sleeps: number[] = [];
    const result = await jevScorer({
      apiKey: "k",
      fetchImpl: fake.fetchImpl,
      sleepImpl: async (ms) => {
        sleeps.push(ms);
      },
    }).score(input());
    expect(sleeps).toEqual([2000, 30_000, 2000]);
    expect(result.fallback).toBeUndefined();
  });

  it("falls back on timeout and does not retry", async () => {
    let attempts = 0;
    const result = await jevScorer({
      apiKey: "k",
      timeoutMs: 20,
      // A hanging request is the one thing jsonFetch cannot script: it
      // must reject only when the deadline's signal aborts.
      fetchImpl: (_url, init) => {
        attempts += 1;
        return new Promise<Response>((_resolve, reject) => {
          init?.signal?.addEventListener(
            "abort",
            () => reject(new Error("aborted")),
            { once: true },
          );
        });
      },
      sleepImpl: async () => {
        throw new Error("timeout must not retry");
      },
    }).score(input());
    expect(result).toEqual({ decisions: [], fallback: "timeout" });
    expect(attempts).toBe(1);
  });

  it("falls back when the api key is missing and does not fetch", async () => {
    const fake = jsonFetch(answerNouls({}));
    const result = await jevScorer({ fetchImpl: fake.fetchImpl }).score(
      input(),
    );
    expect(result).toEqual({
      decisions: [],
      fallback: "no-api-key",
      requested: false,
    });
    expect(fake.requests()).toHaveLength(0);
  });

  it("falls back on invalid JSON and on a non-retryable status", async () => {
    const invalid = await jevScorer({
      apiKey: "k",
      fetchImpl: jsonFetch(() => ({ status: 200, text: "not-json" })).fetchImpl,
    }).score(input());
    expect(invalid).toEqual({ decisions: [], fallback: "invalid-json" });

    const failed = await jevScorer({
      apiKey: "k",
      fetchImpl: jsonFetch(() => ({ status: 500, text: "no" })).fetchImpl,
      sleepImpl: async () => {
        throw new Error("500 must not retry");
      },
    }).score(input());
    expect(failed).toEqual({ decisions: [], fallback: "http:500" });
  });

  it("keeps the answered chunk's decision and usage when another chunk's request fails, and counts the failure", async () => {
    const ok = answerNouls(
      { "seq:1#t1": { reports_failure: 0.9 } },
      { input_tokens: 30, output_tokens: 4 },
    );
    const fake = jsonFetch((request) =>
      sentBody(request).state.chunk.id === "seq:3#t2"
        ? { status: 500, text: "no" }
        : ok(request),
    );
    const result = await jevScorer({
      apiKey: "k",
      fetchImpl: fake.fetchImpl,
    }).score(input([BUILD, LINT]));
    expect(fake.requests()).toHaveLength(2);
    expect(result.fallback).toBeUndefined();
    expect(result.failedRequests).toBe(1);
    expect(result.decisions.map(({ id, tier }) => ({ id, tier }))).toEqual([
      { id: "seq:1#t1", tier: "full" },
    ]);
    expect(result.usage).toEqual({
      inputOther: 30,
      inputCacheRead: 0,
      inputCacheCreation: 0,
      output: 4,
    });
  });

  it("sends no further request once signal aborts and falls back with aborted", async () => {
    const controller = new AbortController();
    const ok = answerNouls({});
    const fake = jsonFetch((request) => {
      controller.abort();
      return ok(request);
    });
    const calls = ["a", "b", "c", "d", "e"].map((id) => ({ ...LINT, id }));
    const result = await jevScorer({
      apiKey: "k",
      fetchImpl: fake.fetchImpl,
    }).score({ ...input(calls), signal: controller.signal });
    expect(fake.requests()).toHaveLength(1);
    expect(result).toEqual({ decisions: [], fallback: "aborted" });
  });

  it("sums input_tokens into usage.inputOther and output_tokens into usage.output", async () => {
    const fake = jsonFetch(
      answerNouls({}, { input_tokens: 120, output_tokens: 7 }),
    );
    const result = await jevScorer({
      apiKey: "k",
      fetchImpl: fake.fetchImpl,
    }).score(input([BUILD, LINT]));
    expect(fake.requests()).toHaveLength(2);
    expect(result.usage).toEqual({
      inputOther: 240,
      inputCacheRead: 0,
      inputCacheCreation: 0,
      output: 14,
    });
  });

  it("omits a chunk whose answers lack a judgment, so it stays as the compiler left it", async () => {
    const fake = jsonFetch(() => ({
      json: { answers: { reports_failure: { type: "noul", noul: 0.9 } } },
    }));
    const result = await jevScorer({
      apiKey: "k",
      fetchImpl: fake.fetchImpl,
    }).score(input());
    expect(result.fallback).toBeUndefined();
    expect(result.decisions).toEqual([]);
  });
});

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

function seed(store: MemoryLogStore): void {
  const append = (payload: unknown) =>
    store.append("s", { kind: "message", payload });
  append({ role: "user", content: [{ type: "text", text: "find the bug" }] });
  for (const [id, file, body] of [
    ["t1", "a.ts", `export function alpha() {}\n${BIG}`],
    ["t2", "b.ts", `export function beta() {}\n${BIG}`],
    ["t3", "c.ts", "CCC"],
  ] as const) {
    append({
      role: "assistant",
      content: [
        {
          type: "tool_use",
          id,
          name: "read",
          input: { path: file },
          rawInput: JSON.stringify({ path: file }),
        },
      ],
    });
    append({
      role: "user",
      content: [
        { type: "tool_result", toolUseId: id, content: body, isError: false },
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

function appendHeaderAndManifest(
  store: MemoryLogStore,
  manifest: unknown,
): void {
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
  store.append("s", { kind: "manifest", payload: manifest });
}

type ScorerReport = { calls: number; usage: Usage; fallback: boolean };

describe("compiler", () => {
  it("sends the progress ledger exactly as the progressLedger claim renders it", async () => {
    const store = new MemoryLogStore();
    seed(store);
    const ledger = progressLedger();
    store.append("s", {
      kind: "message",
      payload: {
        role: "assistant",
        content: [
          {
            type: "tool_use",
            id: "u1",
            name: ledger.tool.def.name,
            input: { goal: "fix the parser", plan: ["read src/parser.ts"] },
            rawInput: "{}",
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
            toolUseId: "u1",
            content: "Ledger updated.",
            isError: false,
          },
        ],
      },
    });
    const fake = jsonFetch(answerNouls({}));
    await contextCompiler({
      budget: 100_000,
      claims: [ledger.provider],
      scorerApply: "entry",
      minScoreTokens: 0,
      scorer: jevScorer({ apiKey: "k", fetchImpl: fake.fetchImpl }),
    }).view(context(store, 1, 1_000_000));

    const ledgers = fake.requests().map((r) => sentBody(r).state.ledger);
    expect(ledgers.length).toBeGreaterThan(0);
    for (const sent of ledgers) {
      expect(sent).toBe("Goal: fix the parser\nPlan:\n- read src/parser.ts");
    }
  });

  it("re-tiers kept chunks on a rebuild from the previous request's original text", async () => {
    const store = new MemoryLogStore();
    seed(store);
    const quiet = contextCompiler({
      budget: 100_000,
      claims: [],
      keepRecent: 1,
    });
    const first = await quiet.view(context(store, 1, 1_000_000));
    appendHeaderAndManifest(store, first.manifest);

    const fake = jsonFetch(
      answerNouls({
        "seq:1#t1": { keep: [0.9] },
        "seq:3#t2": { reports_failure: 0.9 },
      }),
    );
    const second = await contextCompiler({
      budget: 100_000,
      claims: [],
      keepRecent: 1,
      scorer: jevScorer({ apiKey: "k", fetchImpl: fake.fetchImpl }),
    }).view(context(store, 2, 1));

    expect(second.manifest.epochDecision?.rebuild).toBe(true);
    expect(
      fake
        .requests()
        .map((request) => sentBody(request).state.chunk.candidates),
    ).toEqual([
      ["L1: export function alpha() {}"],
      ["L1: export function beta() {}"],
    ]);
    const manifest = second.manifest as typeof second.manifest & {
      scorer?: ScorerReport;
    };
    expect(manifest.scorer).toEqual({
      calls: 1,
      usage: {
        inputOther: 20,
        inputCacheRead: 0,
        inputCacheCreation: 0,
        output: 4,
      },
      fallback: false,
    });
    expect(second.manifest.chunks).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          id: "seq:1#t1",
          tier: "summary",
          summary: "L1: export function alpha() {}",
          reason:
            "scorer:jev:summary:failure=0.00,edit=0.00,lookup=0.00,broad=0.00,keep=0.90",
        }),
        expect.objectContaining({ id: "seq:3#t2", tier: "full" }),
        expect.objectContaining({
          id: "seq:5#t3",
          tier: "full",
          reason: "recent",
        }),
      ]),
    );
  });

  it("records jev's probabilities after the entry reason in entry mode", async () => {
    const store = new MemoryLogStore();
    seed(store);
    const fake = jsonFetch(
      answerNouls({ "seq:5#t3": { is_broad_listing: 0.9 } }),
    );
    const view = await contextCompiler({
      budget: 100_000,
      claims: [],
      scorerApply: "entry",
      minScoreTokens: 0,
      scorer: jevScorer({ apiKey: "k", fetchImpl: fake.fetchImpl }),
    }).view(context(store, 1, 1_000_000));

    expect(fake.requests()).toHaveLength(3);
    expect(view.manifest.chunks).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          id: "seq:5#t3",
          tier: "none",
          reason:
            "scorer:entry:none;scorer:jev:none:failure=0.00,edit=0.00,lookup=0.00,broad=0.90",
        }),
      ]),
    );
  });

  it("records zero rebuild scorer calls when the api key is missing", async () => {
    const store = new MemoryLogStore();
    seed(store);
    const fake = jsonFetch(answerNouls({}));
    const view = await contextCompiler({
      budget: 100_000,
      claims: [],
      keepRecent: 1,
      scorer: jevScorer({ fetchImpl: fake.fetchImpl }),
    }).view(context(store, 1, 1));
    expect(view.manifest.epochDecision?.rebuild).toBe(true);
    const manifest = view.manifest as typeof view.manifest & {
      scorer?: ScorerReport;
    };
    expect(manifest.scorer).toEqual({
      calls: 0,
      usage: {
        inputOther: 0,
        inputCacheRead: 0,
        inputCacheCreation: 0,
        output: 0,
      },
      fallback: true,
    });
    expect(fake.requests()).toHaveLength(0);
  });

  it("records zero shadow calls when the api key is missing and one when it is sent", async () => {
    async function shadowCalls(apiKey?: string) {
      const store = new MemoryLogStore();
      seed(store);
      const fake = jsonFetch(answerNouls({}));
      const view = await contextCompiler({
        budget: 100_000,
        claims: [],
        shadowScorer: jevScorer({
          ...(apiKey ? { apiKey } : {}),
          fetchImpl: fake.fetchImpl,
        }),
      }).view(context(store, 1, 1_000_000));
      store.append("s", { kind: "manifest", payload: view.manifest });
      return {
        sent: fake.requests().length,
        manifestCalls: view.manifest.shadow?.calls,
        healthCalls: runHealth(store.path("s")).shadow.calls,
      };
    }
    expect(await shadowCalls()).toEqual({
      sent: 0,
      manifestCalls: 0,
      healthCalls: 0,
    });
    expect(await shadowCalls("k")).toEqual({
      sent: 3,
      manifestCalls: 1,
      healthCalls: 1,
    });
  });
});
