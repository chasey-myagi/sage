import { MemoryLogStore, type ViewContext } from "@sage/core";
import { listModels } from "@sage/provider";
import type { Request, Usage } from "@sage/provider/contract";
import { expect, it } from "vitest";
import type { Chunk } from "../../src/compiler/chunks.ts";
import { contextCompiler } from "../../src/compiler/compiler.ts";
import { jevScorer } from "../../src/scorer/jev.ts";

const SOURCE = "https://typesafe.ai/blog/introducing-system-one-models-and-jev";

type FetchInit = {
  method?: string;
  headers?: Record<string, string>;
  body?: string;
  signal?: AbortSignal;
};

function chunk(
  id: string,
  tokens = 40,
  input: unknown = { path: "a.ts" },
): Chunk {
  return {
    id,
    kind: "tool",
    messageIndex: 0,
    toolName: "read",
    input,
    tokens,
    accesses: [],
  };
}

function choice(full: number) {
  const none = Number((1 - full).toFixed(4));
  return {
    type: "choice",
    choice: full >= 0.5 ? "full" : "none",
    probabilities: { full, none },
    confidence: Math.max(full, none),
  };
}

function systemOne(
  answers: Record<string, ReturnType<typeof choice>>,
  inputTokens: number,
) {
  return {
    answers,
    usage: { input_tokens: inputTokens, output_tokens: 3 },
  };
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function prevRequest(): Request {
  return {
    model: { provider: "anthropic", model: "claude-sonnet-5" },
    head: { system: "HISTORICAL-SECRET", tools: [] },
    messages: [
      {
        role: "user",
        content: [{ type: "text", text: "HISTORICAL-SECRET" }],
      },
    ],
    cacheBreakpoints: ["head"],
    sampling: {},
  };
}

it("lists jev-latest on the typesafe provider for pricing only", () => {
  expect(listModels("typesafe")).toEqual([
    {
      provider: "typesafe",
      model: "jev-latest",
      contextWindow: 32000,
      pricing: {
        inputPerMTok: 0.042,
        outputPerMTok: 0,
        cacheWritePerMTok: 0,
        cacheReadPerMTok: 0,
        source: SOURCE,
      },
    },
  ]);
});

it("tiers each chunk from probabilities.full and records scorer:jev:<p>", async () => {
  const bodies: {
    model?: string;
    state?: { query?: string; chunks?: { id: string; summary: string }[] };
    questions?: Record<string, unknown>;
  }[] = [];
  const longInput = {
    note: `${"KEEP-".repeat(20)}FULL-ORIGINAL-SECRET`,
  };
  const fetchImpl = async (url: string, init?: FetchInit) => {
    expect(url).toBe("https://api.typesafe.ai/v1/systemone");
    expect(init?.method).toBe("POST");
    expect(init?.headers).toEqual({
      Authorization: "Bearer secret",
      "Content-Type": "application/json",
    });
    const raw = String(init?.body);
    expect(raw).not.toContain("HISTORICAL-SECRET");
    expect(raw).not.toContain("FULL-ORIGINAL-SECRET");
    const body = JSON.parse(raw) as (typeof bodies)[number];
    bodies.push(body);
    return jsonResponse(
      systemOne(
        {
          "seq:1#a": choice(0.5),
          "seq:3#b": choice(0.49),
          "seq:5#c": choice(0.875),
        },
        11,
      ),
    );
  };
  const scorer = jevScorer({ apiKey: "secret", fetchImpl });
  const result = await scorer.score({
    candidates: [
      chunk("seq:1#a"),
      chunk("seq:3#b"),
      chunk("seq:5#c", 40, longInput),
    ],
    prevRequest: prevRequest(),
    query: "find the bug",
  });

  expect(scorer.name).toBe("jev");
  expect(result.fallback).toBeUndefined();
  expect(result.decisions).toEqual([
    { id: "seq:1#a", tier: "full", reason: "scorer:jev:0.50" },
    { id: "seq:3#b", tier: "none", reason: "scorer:jev:0.49" },
    { id: "seq:5#c", tier: "full", reason: "scorer:jev:0.88" },
  ]);
  expect(
    result.decisions.every((decision) => decision.summary === undefined),
  ).toBe(true);
  expect(result.usage).toEqual({
    inputOther: 11,
    inputCacheRead: 0,
    inputCacheCreation: 0,
    output: 0,
  });
  const sent = bodies[0];
  expect(sent?.model).toBe("jev-latest");
  expect(sent?.state?.query).toBe("find the bug");
  expect(sent?.state?.chunks?.[0]).toEqual({
    id: "seq:1#a",
    tool: "read",
    summary: '{"path":"a.ts"}',
    tokens: 40,
  });
  expect(sent?.state?.chunks?.[2]?.summary.length).toBeLessThanOrEqual(80);
  expect(sent?.questions?.["seq:1#a"]).toEqual({
    type: "choice",
    instructions:
      "To finish the task in state.query, will the next step need the full original text of chunk seq:1#a? Answer full if the original text is needed, none if the one-line summary is enough.",
    criteria: {
      full: "The original text is needed.",
      none: "The one-line summary is enough.",
    },
  });

  const strict = await jevScorer({
    apiKey: "secret",
    fetchImpl: async () =>
      jsonResponse(systemOne({ "seq:1#a": choice(0.8) }, 1)),
    minConfidence: 0.9,
    baseUrl: "https://jev.example/",
  }).score({
    candidates: [chunk("seq:1#a")],
    prevRequest: undefined,
    query: "q",
  });
  expect(strict.decisions).toEqual([
    { id: "seq:1#a", tier: "none", reason: "scorer:jev:0.80" },
  ]);
});

it("posts to baseUrl without a trailing slash", async () => {
  let url = "";
  await jevScorer({
    apiKey: "k",
    baseUrl: "https://jev.example/",
    fetchImpl: async (seen) => {
      url = seen;
      return jsonResponse(systemOne({ a: choice(0.1) }, 1));
    },
  }).score({
    candidates: [chunk("a", 1)],
    prevRequest: undefined,
    query: "q",
  });
  expect(url).toBe("https://jev.example/v1/systemone");
});

it("posts opts.model instead of jev-latest", async () => {
  let model = "";
  await jevScorer({
    apiKey: "k",
    model: "served-model",
    fetchImpl: async (_url, init) => {
      const body = JSON.parse(String(init?.body)) as { model?: string };
      model = body.model ?? "";
      return jsonResponse(systemOne({ a: choice(0.9) }, 1));
    },
  }).score({
    candidates: [chunk("a", 1)],
    prevRequest: undefined,
    query: "q",
  });
  expect(model).toBe("served-model");
});

it("splits 200 chunks into at least two batches of at most 40", async () => {
  const sizes: number[] = [];
  const fetchImpl = async (_url: string, init?: FetchInit) => {
    const body = JSON.parse(String(init?.body)) as {
      state: { chunks: { id: string }[] };
    };
    sizes.push(body.state.chunks.length);
    const answers: Record<string, ReturnType<typeof choice>> = {};
    for (const item of body.state.chunks) answers[item.id] = choice(0.1);
    return jsonResponse(systemOne(answers, 1));
  };
  const candidates = Array.from({ length: 200 }, (_, index) =>
    chunk(`seq:${index}#c`, 10),
  );
  const result = await jevScorer({ apiKey: "k", fetchImpl }).score({
    candidates,
    prevRequest: undefined,
    query: "q",
  });
  expect(sizes.length).toBeGreaterThanOrEqual(2);
  expect(sizes.every((size) => size <= 40)).toBe(true);
  expect(sizes.reduce((sum, size) => sum + size, 0)).toBe(200);
  expect(result.decisions).toHaveLength(200);
  expect(result.usage?.inputOther).toBe(sizes.length);
});

it("starts a new batch when token-estimated state characters pass 24000", async () => {
  const sizes: number[] = [];
  const fetchImpl = async (_url: string, init?: FetchInit) => {
    const body = JSON.parse(String(init?.body)) as {
      state: { chunks: unknown[] };
    };
    sizes.push(body.state.chunks.length);
    return jsonResponse(systemOne({ a: choice(0.1), b: choice(0.1) }, 1));
  };
  await jevScorer({ apiKey: "k", fetchImpl, batchSize: 40 }).score({
    candidates: [chunk("a", 3000), chunk("b", 3000)],
    prevRequest: undefined,
    query: "qq",
  });
  expect(sizes).toEqual([1, 1]);

  sizes.length = 0;
  await jevScorer({ apiKey: "k", fetchImpl, batchSize: 40 }).score({
    candidates: [chunk("a", 10_000)],
    prevRequest: undefined,
    query: "qq",
  });
  expect(sizes).toEqual([1]);
});

it("retries a 429 twice with exponential backoff and then succeeds", async () => {
  const statuses = [429, 429, 200];
  let attempts = 0;
  const sleeps: number[] = [];
  const fetchImpl = async () => {
    const status = statuses[attempts] ?? 200;
    attempts += 1;
    if (status !== 200) return new Response("busy", { status });
    return jsonResponse(systemOne({ "seq:1#a": choice(0.7) }, 8));
  };
  const result = await jevScorer({
    apiKey: "k",
    fetchImpl,
    sleepImpl: async (ms) => {
      sleeps.push(ms);
    },
  }).score({
    candidates: [chunk("seq:1#a")],
    prevRequest: undefined,
    query: "q",
  });
  expect(attempts).toBe(3);
  expect(sleeps).toEqual([500, 1000]);
  expect(result.fallback).toBeUndefined();
  expect(result.decisions).toEqual([
    { id: "seq:1#a", tier: "full", reason: "scorer:jev:0.70" },
  ]);
});

it("retries 529 and stops after three failed retries", async () => {
  let attempts = 0;
  const sleeps: number[] = [];
  const once = await jevScorer({
    apiKey: "k",
    fetchImpl: async () => {
      attempts += 1;
      if (attempts === 1) return new Response("overloaded", { status: 529 });
      return jsonResponse(systemOne({ a: choice(0.2) }, 2));
    },
    sleepImpl: async (ms) => {
      sleeps.push(ms);
    },
  }).score({
    candidates: [chunk("a", 1)],
    prevRequest: undefined,
    query: "q",
  });
  expect(attempts).toBe(2);
  expect(sleeps).toEqual([500]);
  expect(once.decisions).toEqual([
    { id: "a", tier: "none", reason: "scorer:jev:0.20" },
  ]);

  attempts = 0;
  sleeps.length = 0;
  const exhausted = await jevScorer({
    apiKey: "k",
    fetchImpl: async () => {
      attempts += 1;
      return new Response("busy", { status: 429 });
    },
    sleepImpl: async (ms) => {
      sleeps.push(ms);
    },
  }).score({
    candidates: [chunk("a", 1)],
    prevRequest: undefined,
    query: "q",
  });
  expect(attempts).toBe(4);
  expect(sleeps).toEqual([500, 1000, 2000]);
  expect(exhausted).toEqual({ decisions: [], fallback: "http:429" });
});

it("falls back on timeout and does not retry", async () => {
  let attempts = 0;
  const fetchImpl = (_url: string, init?: FetchInit) => {
    attempts += 1;
    return new Promise<Response>((resolve, reject) => {
      const timer = setTimeout(() => resolve(jsonResponse({})), 5_000);
      const abort = () => {
        clearTimeout(timer);
        reject(Object.assign(new Error("aborted"), { name: "AbortError" }));
      };
      if (init?.signal?.aborted) abort();
      else init?.signal?.addEventListener("abort", abort, { once: true });
    });
  };
  const result = await jevScorer({
    apiKey: "k",
    fetchImpl,
    timeoutMs: 20,
    sleepImpl: async () => {
      throw new Error("timeout must not retry");
    },
  }).score({
    candidates: [chunk("a", 1)],
    prevRequest: undefined,
    query: "q",
  });
  expect(result).toEqual({ decisions: [], fallback: "timeout" });
  expect(attempts).toBe(1);
});

it("falls back when the api key is missing and does not fetch", async () => {
  let attempts = 0;
  const result = await jevScorer({
    fetchImpl: async () => {
      attempts += 1;
      return jsonResponse({});
    },
  }).score({
    candidates: [chunk("a", 1)],
    prevRequest: undefined,
    query: "q",
  });
  expect(result).toEqual({
    decisions: [],
    fallback: "no-api-key",
    requested: false,
  });
  expect(attempts).toBe(0);
});

it("records zero rebuild scorer calls when the api key is missing", async () => {
  const store = new MemoryLogStore();
  seed(store);
  let attempts = 0;
  const view = await contextCompiler({
    budget: 100_000,
    claims: [],
    keepRecent: 1,
    scorer: jevScorer({
      fetchImpl: async () => {
        attempts += 1;
        return jsonResponse({});
      },
    }),
  }).view(context(store));

  expect(view.manifest.epochDecision?.rebuild).toBe(true);
  const manifest = view.manifest as typeof view.manifest & {
    scorer?: { calls: number; usage: Usage; fallback: boolean };
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
  expect(attempts).toBe(0);
});

it("falls back on invalid JSON and on a non-retryable status", async () => {
  const invalid = await jevScorer({
    apiKey: "k",
    fetchImpl: async () => new Response("not-json", { status: 200 }),
  }).score({
    candidates: [chunk("a", 1)],
    prevRequest: undefined,
    query: "q",
  });
  expect(invalid).toEqual({ decisions: [], fallback: "invalid-json" });

  const missing = await jevScorer({
    apiKey: "k",
    fetchImpl: async () => new Response("no", { status: 500 }),
    sleepImpl: async () => {
      throw new Error("500 must not retry");
    },
  }).score({
    candidates: [chunk("a", 1)],
    prevRequest: undefined,
    query: "q",
  });
  expect(missing).toEqual({ decisions: [], fallback: "http:500" });
});

it("omits a chunk with no answer so it stays kept", async () => {
  const result = await jevScorer({
    apiKey: "k",
    fetchImpl: async () =>
      jsonResponse(
        systemOne(
          {
            "seq:1#a": choice(0.1),
          },
          4,
        ),
      ),
  }).score({
    candidates: [chunk("seq:1#a"), chunk("seq:3#b")],
    prevRequest: undefined,
    query: "q",
  });
  expect(result.fallback).toBeUndefined();
  expect(result.decisions).toEqual([
    { id: "seq:1#a", tier: "none", reason: "scorer:jev:0.10" },
  ]);
});

it("sums each batch input_tokens into usage.inputOther", async () => {
  const tokens = [10, 25];
  let index = 0;
  const result = await jevScorer({
    apiKey: "k",
    batchSize: 1,
    fetchImpl: async (_url, init) => {
      const body = JSON.parse(String(init?.body)) as {
        state: { chunks: { id: string }[] };
      };
      const id = body.state.chunks[0]?.id ?? "";
      const inputTokens = tokens[index] ?? 0;
      index += 1;
      return jsonResponse(systemOne({ [id]: choice(0.6) }, inputTokens));
    },
  }).score({
    candidates: [chunk("a", 1), chunk("b", 1)],
    prevRequest: undefined,
    query: "q",
  });
  expect(result.usage).toEqual({
    inputOther: 35,
    inputCacheRead: 0,
    inputCacheCreation: 0,
    output: 0,
  });
  expect(result.decisions.map((decision) => decision.tier)).toEqual([
    "full",
    "full",
  ]);
});

it("drops earlier decisions when a later batch fails", async () => {
  let index = 0;
  const result = await jevScorer({
    apiKey: "k",
    batchSize: 1,
    fetchImpl: async () => {
      index += 1;
      if (index === 1) return jsonResponse(systemOne({ a: choice(0.9) }, 4));
      return new Response("no", { status: 500 });
    },
  }).score({
    candidates: [chunk("a", 1), chunk("b", 1)],
    prevRequest: undefined,
    query: "q",
  });
  expect(result).toEqual({ decisions: [], fallback: "http:500" });
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

function toolMessage(id: string, file: string) {
  return {
    role: "assistant" as const,
    content: [
      {
        type: "tool_use" as const,
        id,
        name: "read",
        input: { path: file },
        rawInput: JSON.stringify({ path: file }),
      },
    ],
  };
}

function resultMessage(id: string, content: string) {
  return {
    role: "user" as const,
    content: [
      {
        type: "tool_result" as const,
        toolUseId: id,
        content,
        isError: false,
      },
    ],
  };
}

function seed(store: MemoryLogStore): void {
  store.append("s", {
    kind: "message",
    payload: {
      role: "user",
      content: [{ type: "text", text: "find the bug" }],
    },
  });
  store.append("s", { kind: "message", payload: toolMessage("t1", "a.ts") });
  store.append("s", { kind: "message", payload: resultMessage("t1", "AAA") });
  store.append("s", { kind: "message", payload: toolMessage("t2", "b.ts") });
  store.append("s", { kind: "message", payload: resultMessage("t2", "BBB") });
  store.append("s", { kind: "message", payload: toolMessage("t3", "c.ts") });
  store.append("s", { kind: "message", payload: resultMessage("t3", "CCC") });
}

function context(store: MemoryLogStore): ViewContext {
  return {
    sessionId: "s",
    turn: 1,
    path: store.path("s"),
    model: MODEL,
    prepared: {
      model: MODEL,
      contextWindow: 1,
      pricing: null,
      adapterDefaults: {},
    },
    head: {
      system: HEAD.system,
      tools: HEAD.tools.map((tool) => ({ ...tool })),
    },
    tools: [
      {
        def: HEAD.tools[0] ?? {
          name: "read",
          description: "read a file",
          inputSchema: { type: "object" },
        },
        isReadOnly: true,
        isDestructive: false,
        isOpenWorld: false,
        accesses(input: unknown) {
          if (
            typeof input !== "object" ||
            input === null ||
            !("path" in input)
          ) {
            return [];
          }
          const key = (input as { path?: unknown }).path;
          return typeof key === "string"
            ? [{ key, mode: "read" as const }]
            : [];
        },
        async execute() {
          return { content: "", isError: false };
        },
      },
    ],
  };
}

function resultContent(
  messages: {
    content: { type: string; toolUseId?: string; content?: string }[];
  }[],
  id: string,
): string | undefined {
  for (const message of messages) {
    for (const block of message.content) {
      if (block.type === "tool_result" && block.toolUseId === id) {
        return block.content;
      }
    }
  }
  return undefined;
}

it("counts manifest.scorer and leaves an unanswered chunk kept", async () => {
  const store = new MemoryLogStore();
  seed(store);
  const view = await contextCompiler({
    budget: 100_000,
    claims: [],
    keepRecent: 1,
    scorer: jevScorer({
      apiKey: "k",
      fetchImpl: async () =>
        jsonResponse(
          systemOne(
            {
              "seq:1#t1": choice(0.2),
            },
            15,
          ),
        ),
    }),
  }).view(context(store));

  expect(view.manifest.epochDecision?.rebuild).toBe(true);
  const manifest = view.manifest as typeof view.manifest & {
    scorer?: { calls: number; usage: Usage; fallback: boolean };
  };
  expect(manifest.scorer).toEqual({
    calls: 1,
    usage: {
      inputOther: 15,
      inputCacheRead: 0,
      inputCacheCreation: 0,
      output: 0,
    },
    fallback: false,
  });
  expect(view.manifest.chunks).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        id: "seq:1#t1",
        tier: "none",
        reason: "scorer:jev:0.20",
      }),
      expect.objectContaining({
        id: "seq:3#t2",
        tier: "full",
        reason: "kept",
      }),
      expect.objectContaining({
        id: "seq:5#t3",
        tier: "full",
        reason: "recent",
      }),
    ]),
  );
  expect(resultContent(view.messages, "t1")).not.toBe("AAA");
  expect(resultContent(view.messages, "t1")).not.toContain(
    "summary (model notes",
  );
  expect(resultContent(view.messages, "t2")).toBe("BBB");
});
