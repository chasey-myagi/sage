import { MemoryLogStore, passthroughView, type ViewContext } from "@sage/core";
import { expect, it } from "vitest";
import { defaultTokenCount } from "../../src/compiler/budget.ts";
import type { Claim, ClaimProvider } from "../../src/compiler/claim.ts";
import { contextCompiler } from "../../src/compiler/compiler.ts";

const MODEL = { provider: "anthropic", model: "claude-test" };
const HEAD = { system: "You are sage.", tools: [] as const };

function context(
  store: MemoryLogStore,
  turn = 1,
  sessionId = "s",
): ViewContext {
  return {
    sessionId,
    turn,
    path: store.path(sessionId),
    model: MODEL,
    prepared: {
      model: MODEL,
      contextWindow: 8000,
      pricing: null,
      adapterDefaults: {},
    },
    head: { system: HEAD.system, tools: [...HEAD.tools] },
    tools: [],
  };
}

function user(text: string) {
  return {
    role: "user" as const,
    content: [{ type: "text" as const, text }],
  };
}

function render(id: string, tier: string, body: string): string {
  return `<claim id="${id}" tier="${tier}">\n${body}\n</claim>`;
}

function countOf(table: Record<string, number>) {
  return (text: string) => table[text] ?? defaultTokenCount(text);
}

function claimHash(
  claims: { id: string; contentHash?: string; reason: string }[],
  id: string,
): string | undefined {
  return claims.find((claim) => claim.id === id)?.contentHash;
}

it("writes downgrade reasons and the budget object onto the manifest", async () => {
  const compiler = contextCompiler({
    budget: 10000,
    count: countOf({
      "a-full": 3000,
      "a-line": 100,
      "b-full": 9000,
      "b-line": 100,
      "c-full": 6000,
      "c-line": 100,
    }),
    claims: [
      () => [
        {
          id: "a",
          priority: 90,
          placement: "tail",
          tiers: { full: "a-full", line: "a-line" },
        },
        {
          id: "b",
          priority: 50,
          placement: "tail",
          tiers: { full: "b-full", line: "b-line" },
        },
        {
          id: "c",
          priority: 10,
          placement: "tail",
          tiers: { full: "c-full", line: "c-line" },
        },
      ],
    ],
  });
  const store = new MemoryLogStore();
  store.append("s", { kind: "message", payload: user("hello") });
  const view = await compiler.view(context(store));

  expect(
    view.manifest.claims.map((record) => ({
      id: record.id,
      tier: record.tier,
      reason: record.reason,
      tokens: record.tokens,
    })),
  ).toEqual([
    { id: "a", tier: "full", reason: "fits", tokens: 3000 },
    { id: "b", tier: "line", reason: "budget:downgraded", tokens: 100 },
    { id: "c", tier: "line", reason: "budget:downgraded", tokens: 100 },
  ]);
  expect(view.manifest.budget).toEqual({
    total: 10000,
    used: 3202,
    overrun: 0,
    omitted: 0,
    expands: 0,
    frozenTokens: 0,
    staleOmittable: 0,
  });
  expect(view.manifest.chunks).toEqual([
    { id: "seq:0", tier: "full", reason: "kept", tokens: 2 },
  ]);
});

it("keeps a pinned claim and records the overrun when the pin exceeds the budget", async () => {
  const compiler = contextCompiler({
    budget: 10000,
    count: countOf({ pin: 20000, "x-full": 5000, "x-line": 100 }),
    claims: [
      () => [
        {
          id: "pin",
          priority: 100,
          pinned: true,
          placement: "epoch-head",
          tiers: { full: "pin" },
        },
        {
          id: "x",
          priority: 10,
          placement: "tail",
          tiers: { full: "x-full", line: "x-line" },
        },
      ],
    ],
  });
  const store = new MemoryLogStore();
  store.append("s", { kind: "message", payload: user("hello") });
  const view = await compiler.view(context(store));

  expect(view.manifest.claims).toEqual([
    expect.objectContaining({
      id: "pin",
      tier: "full",
      reason: "pinned",
      tokens: 20000,
    }),
    expect.objectContaining({
      id: "x",
      tier: "none",
      reason: "budget:downgraded",
      tokens: 0,
    }),
  ]);
  expect(view.manifest.budget).toEqual({
    total: 10000,
    used: 20002,
    overrun: 10002,
    omitted: 0,
    expands: 0,
    frozenTokens: 0,
    staleOmittable: 0,
  });
  expect(JSON.stringify(view.messages)).not.toContain("x-full");
  expect(JSON.stringify(view.messages)).not.toContain("x-line");
});

it("freezes the epoch-head message for later turns of the same epoch", async () => {
  let body = "first";
  const claims: ClaimProvider = () => [
    {
      id: "persona",
      priority: 80,
      placement: "epoch-head",
      tiers: { full: body },
    },
    {
      id: "later",
      priority: 10,
      placement: "epoch-head",
      tiers: { full: "second-claim" },
    },
  ];
  const compiler = contextCompiler({ budget: 10000, claims: [claims] });
  const store = new MemoryLogStore();
  store.append("s", { kind: "message", payload: user("hello") });

  const first = await compiler.view(context(store, 1));
  const headText = [
    render("persona", "full", "first"),
    render("later", "full", "second-claim"),
  ].join("\n\n");
  expect(first.messages[0]).toEqual({
    role: "user",
    content: [{ type: "text", text: headText }],
  });
  expect(first.messages[1]).toEqual(user("hello"));
  expect(first.epochStart).toEqual({ reason: "compiler:first-turn" });
  expect(first.manifest.chunks).toHaveLength(1);
  expect(first.manifest.chunks[0]).toMatchObject({
    tier: "full",
    reason: "kept",
  });

  store.append("s", {
    kind: "view_epoch",
    payload: { epoch: 0, reason: "compiler:first-turn" },
    details: { epochHead: structuredClone(first.messages[0]) },
  });
  body = "CHANGED";
  const second = await compiler.view(context(store, 2));
  const third = await compiler.view(context(store, 3));

  expect(second.messages[0]).toEqual(first.messages[0]);
  expect(third.messages[0]).toEqual(first.messages[0]);
  expect(JSON.stringify(second.messages[0])).not.toContain("CHANGED");
  expect(second.epochStart).toBeUndefined();
  expect(third.epochStart).toBeUndefined();
  expect(second.manifest.claims.map((record) => record.reason)).toEqual([
    "frozen:epoch-head",
    "frozen:epoch-head",
  ]);
});

it("appends tail claims onto the last user message and recomputes them every turn", async () => {
  let body = "one";
  const compiler = contextCompiler({
    budget: 10000,
    claims: [
      (): Claim[] => [
        {
          id: "low",
          priority: 1,
          placement: "tail",
          tiers: { full: "low-body" },
        },
        {
          id: "high",
          priority: 9,
          placement: "tail",
          tiers: { full: body },
        },
      ],
    ],
  });
  const store = new MemoryLogStore();
  store.append("s", { kind: "message", payload: user("hello") });
  const first = await compiler.view(context(store, 1));
  expect(first.messages).toHaveLength(1);
  expect(first.messages[0]?.content).toEqual([
    { type: "text", text: "hello" },
    { type: "text", text: render("high", "full", "one") },
    { type: "text", text: render("low", "full", "low-body") },
  ]);

  body = "two";
  const second = await compiler.view(context(store, 2));
  expect(second.messages[0]?.content[1]).toEqual({
    type: "text",
    text: render("high", "full", "two"),
  });
});

it("opens a new user message when the history does not end on user", async () => {
  const compiler = contextCompiler({
    budget: 10000,
    claims: [
      () => [
        {
          id: "note",
          priority: 1,
          placement: "tail",
          tiers: { full: "tail-body" },
        },
      ],
    ],
  });
  const store = new MemoryLogStore();
  store.append("s", { kind: "message", payload: user("hello") });
  store.append("s", {
    kind: "message",
    payload: {
      role: "assistant",
      content: [{ type: "text", text: "ok" }],
    },
  });
  const view = await compiler.view(context(store));
  expect(view.messages.at(-1)).toEqual({
    role: "user",
    content: [{ type: "text", text: render("note", "full", "tail-body") }],
  });
  expect(view.messages).toHaveLength(3);
});

it("keeps tail-if-changed text in the view but logs the body only when the hash changes", async () => {
  let body = "alpha";
  const compiler = contextCompiler({
    budget: 10000,
    claims: [
      () => [
        {
          id: "ledger",
          priority: 70,
          placement: "tail-if-changed",
          tiers: { full: body },
        },
      ],
    ],
  });
  const store = new MemoryLogStore();
  store.append("s", { kind: "message", payload: user("hello") });
  const rounds = [];
  for (let turn = 1; turn <= 4; turn += 1) {
    if (turn === 4) body = "beta";
    const view = await compiler.view(context(store, turn));
    rounds.push(view);
    store.append("s", {
      kind: "manifest",
      payload: view.manifest,
      ...(view.details ? { details: view.details } : {}),
    });
  }

  const hashes = rounds.map((view) =>
    claimHash(
      view.manifest.claims as {
        id: string;
        contentHash?: string;
        reason: string;
      }[],
      "ledger",
    ),
  );
  expect(hashes[0]).toBeTruthy();
  expect(hashes[1]).toBe(hashes[0]);
  expect(hashes[2]).toBe(hashes[0]);
  expect(hashes[3]).not.toBe(hashes[0]);

  const content = (view: (typeof rounds)[number]) =>
    (view.details?.claimContent as Record<string, string> | undefined)?.ledger;
  expect(content(rounds[0])).toBe(render("ledger", "full", "alpha"));
  expect(content(rounds[1])).toBeUndefined();
  expect(content(rounds[2])).toBeUndefined();
  expect(content(rounds[3])).toBe(render("ledger", "full", "beta"));
  expect(rounds.map((view) => view.manifest.claims[0]?.reason)).toEqual([
    "fits",
    "unchanged",
    "unchanged",
    "changed",
  ]);
  expect(JSON.stringify(rounds[2]?.messages)).toContain("alpha");
  expect(JSON.stringify(rounds[3]?.messages)).toContain("beta");
  expect(JSON.stringify(rounds[3]?.messages)).not.toContain("alpha");
});

it("matches passthrough messages when no claim is submitted and records chunk decisions", async () => {
  const store = new MemoryLogStore();
  store.append("s", { kind: "message", payload: user("go") });
  store.append("s", {
    kind: "message",
    payload: {
      role: "assistant",
      content: [
        {
          type: "tool_use",
          id: "call_9",
          name: "bash",
          input: { cmd: "ls" },
          rawInput: '{"cmd":"ls"}',
        },
      ],
    },
  });
  const ctx = context(store);
  const plain = passthroughView.view(ctx);
  const compiled = await contextCompiler({ budget: 8000, claims: [] }).view(
    ctx,
  );

  expect(compiled.epoch).toBe(plain.epoch);
  expect(compiled.head).toEqual(plain.head);
  expect(compiled.messages).toEqual(plain.messages);
  const last = compiled.messages.length - 1;
  expect(compiled.cacheBreakpoints).toEqual([
    "head",
    { afterMessage: last },
    { afterMessage: last },
  ]);
  expect(compiled.epochStart).toBeUndefined();
  expect(compiled.details).toBeUndefined();
  const {
    budget,
    chunks,
    epochDecision: _epochDecision,
    calibration: _calibration,
    cache: _cache,
    ...rest
  } = compiled.manifest;
  const {
    budget: plainBudget,
    chunks: plainChunks,
    ...plainRest
  } = plain.manifest;
  expect(plainBudget).toBeUndefined();
  expect(plainChunks.map((chunk) => chunk.reason)).toEqual([
    "passthrough",
    "passthrough",
    "passthrough",
  ]);
  expect(rest).toEqual(plainRest);
  expect(chunks.map(({ id, tier, reason }) => ({ id, tier, reason }))).toEqual([
    { id: "seq:0", tier: "full", reason: "kept" },
    { id: "seq:1#call_9", tier: "full", reason: "recent" },
  ]);
  const kept = chunks.reduce(
    (sum, chunk) => sum + (chunk.tier === "none" ? 0 : (chunk.tokens ?? 0)),
    0,
  );
  expect(budget).toEqual({
    total: 8000,
    used: kept,
    overrun: 0,
    omitted: 0,
    expands: 0,
    frozenTokens: 0,
    staleOmittable: 0,
  });
});

it("records a thrown provider and still renders the other claims", async () => {
  const compiler = contextCompiler({
    budget: 10000,
    claims: [
      () => {
        throw new Error("boom");
      },
      () => [
        {
          id: "ok",
          priority: 1,
          placement: "tail",
          tiers: { full: "visible" },
        },
      ],
    ],
  });
  const store = new MemoryLogStore();
  store.append("s", { kind: "message", payload: user("hello") });
  const view = await compiler.view(context(store));

  expect(view.manifest.claims).toContainEqual(
    expect.objectContaining({
      id: "provider:0",
      tier: "none",
      reason: "provider_error:boom",
      tokens: 0,
    }),
  );
  const texts = view.messages.flatMap((message) =>
    message.content.flatMap((block) =>
      block.type === "text" ? [block.text] : [],
    ),
  );
  expect(texts).toContain(render("ok", "full", "visible"));
  expect(texts.join("\n")).not.toContain("boom");
});

it("uses ceil(length / 4) when no token counter is provided", async () => {
  const compiler = contextCompiler({
    budget: 10000,
    claims: [
      () => [
        {
          id: "n",
          priority: 1,
          placement: "tail",
          tiers: { full: "abcdefghij" },
        },
      ],
    ],
  });
  const store = new MemoryLogStore();
  store.append("s", { kind: "message", payload: user("hello") });
  const view = await compiler.view(context(store));
  expect(view.manifest.claims[0]?.tokens).toBe(3);
});
