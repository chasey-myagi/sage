import { MemoryLogStore } from "@sage/core";
import { renderLlmInput } from "@sage/core/testing";
import { AnthropicAdapter } from "@sage/provider";
import type { Message, Request } from "@sage/provider/contract";
import { expect, it } from "vitest";
import { contextCompiler } from "../../src/compiler/compiler.ts";
import { frozenDecisions } from "../../src/compiler/freeze.ts";

const MODEL = { provider: "anthropic", model: "claude-sonnet-5" };
const HEAD = { system: "You are sage.", tools: [] as const };
const SSE = [
  "event: message_start",
  'data: {"type":"message_start","message":{"id":"msg_01","type":"message","role":"assistant","model":"claude-sonnet-5","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":1}}}',
  "",
  "event: content_block_start",
  'data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}',
  "",
  "event: content_block_delta",
  'data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}',
  "",
  "event: content_block_stop",
  'data: {"type":"content_block_stop","index":0}',
  "",
  "event: message_delta",
  'data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":1}}',
  "",
  "event: message_stop",
  'data: {"type":"message_stop"}',
  "",
].join("\n");

function context(store: MemoryLogStore, turn: number) {
  return {
    sessionId: "s",
    turn,
    path: store.path("s"),
    model: MODEL,
    prepared: {
      model: MODEL,
      contextWindow: 1_000_000,
      pricing: {
        inputPerMTok: 2,
        outputPerMTok: 10,
        cacheWritePerMTok: 2.5,
        cacheReadPerMTok: 0.2,
        source: "https://platform.claude.com/docs/en/about-claude/pricing",
      },
      adapterDefaults: {},
    },
    head: { system: HEAD.system, tools: [...HEAD.tools] },
    tools: [],
  };
}

function user(text: string): Message {
  return { role: "user", content: [{ type: "text", text }] };
}

function assistantText(text: string): Message {
  return { role: "assistant", content: [{ type: "text", text }] };
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
          name: "bash",
          input: { id },
          rawInput: `{"id":"${id}"}`,
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

function resultText(
  messages: readonly Message[],
  toolUseId: string,
): string | undefined {
  for (const message of messages) {
    for (const block of message.content) {
      if (block.type === "tool_result" && block.toolUseId === toolUseId) {
        return block.content;
      }
    }
  }
  return undefined;
}

function requestOf(
  messages: Message[],
  cacheBreakpoints: Request["cacheBreakpoints"],
): Request {
  return {
    model: MODEL,
    head: { system: HEAD.system, tools: [] },
    messages,
    cacheBreakpoints,
    sampling: {},
  };
}

function cacheControlPaths(value: unknown, path: string, acc: string[]): void {
  if (Array.isArray(value)) {
    for (let index = 0; index < value.length; index += 1) {
      cacheControlPaths(value[index], `${path}[${index}]`, acc);
    }
    return;
  }
  if (value === null || typeof value !== "object") return;
  const record = value as Record<string, unknown>;
  if ("cache_control" in record) acc.push(path);
  for (const key of Object.keys(record)) {
    cacheControlPaths(record[key], path === "" ? key : `${path}.${key}`, acc);
  }
}

it("returns undefined until the epoch has a manifest", () => {
  const store = new MemoryLogStore();
  expect(frozenDecisions(store.path("s"), 0)).toBeUndefined();
  store.append("s", {
    kind: "view_epoch",
    payload: { epoch: 0, reason: "compiler:first-turn" },
  });
  expect(frozenDecisions(store.path("s"), 0)).toBeUndefined();
  store.append("s", {
    kind: "manifest",
    payload: {
      epoch: 0,
      turn: 1,
      claims: [],
      chunks: [
        { id: "seq:3#x", tier: "none", reason: "budget:oldest-first" },
        { id: "seq:5#b", tier: "full", reason: "recent" },
      ],
      anomalies: [],
    },
  });
  const frozen = frozenDecisions(store.path("s"), 0);
  expect(frozen?.get("seq:3#x")).toEqual({
    id: "seq:3#x",
    tier: "none",
    reason: "budget:oldest-first",
  });
  expect(frozen?.get("seq:5#b")?.tier).toBe("full");
});

it("freezes chunk tiers and stub bytes inside an epoch and only appends", async () => {
  const compiler = contextCompiler({
    budget: 1,
    claims: [],
    expectedTurns: 1,
    count: () => 100,
  });
  const store = new MemoryLogStore();
  store.append("s", { kind: "message", payload: user("preface") });
  store.append("s", { kind: "message", payload: assistantText("note") });
  store.append("s", { kind: "message", payload: user("go") });
  appendTool(store, "x", "KEEP-A");
  appendTool(store, "b", "KEEP-B");
  for (let index = 0; index < 5; index += 1) {
    appendTool(store, `t${index}`, "KEEP");
  }

  const views = [];
  for (let turn = 1; turn <= 5; turn += 1) {
    if (turn === 2 || turn === 3) {
      appendTool(store, `n${turn}a`, "NEW");
      appendTool(store, `n${turn}b`, "NEW");
    }
    if (turn === 4) appendTool(store, "n4", "NEW");
    if (turn === 5) {
      store.append("s", { kind: "message", payload: user("later") });
      store.append("s", { kind: "message", payload: assistantText("ack") });
    }
    const view = await compiler.view(context(store, turn));
    views.push(view);
    store.append("s", { kind: "manifest", payload: view.manifest });
  }

  const first = views[0];
  if (!first) throw new Error("missing turn 1");
  const chunkA = first.manifest.chunks.find((chunk) => chunk.id === "seq:3#x");
  const chunkB = first.manifest.chunks.find((chunk) => chunk.id.endsWith("#b"));
  expect(chunkA).toMatchObject({ tier: "none" });
  expect(chunkB).toMatchObject({ tier: "full" });
  const stubA = resultText(first.messages, "x");
  expect(stubA).toBeTruthy();
  expect(stubA).not.toBe("KEEP-A");

  const snapshots: string[] = [];
  for (let index = 1; index < views.length; index += 1) {
    const view = views[index];
    const previous = views[index - 1];
    if (!view || !previous) throw new Error(`missing turn ${index + 1}`);
    expect(view.epoch).toBe(0);
    expect(view.epochStart).toBeUndefined();
    expect(
      view.manifest.chunks.find((chunk) => chunk.id === "seq:3#x"),
    ).toMatchObject({
      tier: "none",
      reason: chunkA?.reason,
    });
    expect(
      view.manifest.chunks.find((chunk) => chunk.id.endsWith("#b")),
    ).toMatchObject({
      tier: "full",
    });
    expect(resultText(view.messages, "x")).toBe(stubA);
    expect(resultText(view.messages, "b")).toBe("KEEP-B");
    const toolIds = view.manifest.chunks
      .map((chunk) => chunk.id)
      .filter((id) => id.includes("#"));
    const bIndex = toolIds.findIndex((id) => id.endsWith("#b"));
    expect(toolIds.length - 1 - bIndex).toBeGreaterThanOrEqual(6);
    const rendered = renderLlmInput(
      requestOf(view.messages, view.cacheBreakpoints),
      requestOf(previous.messages, previous.cacheBreakpoints),
    );
    snapshots.push(rendered);
    expect(
      rendered.startsWith("system: <same>\ntools: <same>\nmessages:\n<last>\n"),
    ).toBe(true);
    expect(rendered).toContain("\n+ ");
  }
  expect(snapshots).toHaveLength(4);

  const later = views[1];
  if (!later) throw new Error("missing turn 2");
  const prefixEnd = first.messages.length - 1;
  const last = later.messages.length - 1;
  expect(prefixEnd).toBeLessThan(last);
  expect(later.cacheBreakpoints).toEqual([
    "head",
    { afterMessage: prefixEnd },
    { afterMessage: last },
  ]);

  const calls: { body: unknown }[] = [];
  const fetchImpl: typeof fetch = async (_url, init) => {
    calls.push({ body: JSON.parse(String(init?.body)) });
    return new Response(SSE, {
      status: 200,
      headers: { "content-type": "text/event-stream" },
    });
  };
  const adapter = new AnthropicAdapter({ apiKey: "k", fetchImpl });
  const request = requestOf(later.messages, later.cacheBreakpoints);
  for await (const _chunk of adapter.stream(request, adapter.prepare(MODEL))) {
    // drain
  }
  const paths: string[] = [];
  cacheControlPaths(calls[0]?.body, "", paths);
  expect(paths).toHaveLength(3);
});
