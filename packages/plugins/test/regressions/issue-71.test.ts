import { stopScript } from "@sage/core/testing";
import type {
  ProviderAdapter,
  Request,
  StreamChunk,
  Usage,
} from "@sage/provider/contract";
import { expect, it } from "vitest";
import type { Chunk } from "../../src/compiler/chunks.ts";
import { modelScorer } from "../../src/scorer/model.ts";

const USAGE: Usage = {
  inputOther: 11,
  inputCacheRead: 80,
  inputCacheCreation: 2,
  output: 7,
};

function chunk(id: string): Chunk {
  return {
    id,
    kind: "tool",
    messageIndex: 0,
    toolUseId: id.slice(id.indexOf("#") + 1),
    toolName: "bash",
    input: { cmd: "git log" },
    tokens: 12,
    accesses: [],
  };
}

function prevRequest(model: { provider: string; model: string }): Request {
  return {
    model,
    head: { system: "You are sage.", tools: [] },
    messages: [{ role: "user", content: [{ type: "text", text: "hello" }] }],
    cacheBreakpoints: ["head"],
    sampling: { temperature: 0 },
  };
}

function scripted(script: StreamChunk[]): ProviderAdapter {
  return {
    api: "anthropic-messages",
    prepare(model) {
      return {
        model,
        contextWindow: 8000,
        pricing: null,
        adapterDefaults: {},
      };
    },
    stream() {
      return (async function* () {
        for (const item of script) yield item;
      })();
    },
  };
}

async function score(text: string, candidates: Chunk[]) {
  const model = { provider: "anthropic", model: "claude-haiku-4-5" };
  return modelScorer({
    adapters: { anthropic: scripted(stopScript(text, USAGE)) },
    model,
  }).score({
    candidates,
    prevRequest: prevRequest(model),
    query: "find the bug",
  });
}

const CANDIDATE = "seq:39#call_00_git";

it("parses one decision when a JSON array is followed by explanation", async () => {
  const text = [
    `[{"id":"${CANDIDATE}","tier":"summary","summary":"git log shows the recent commits"}]`,
    "",
    "The candidates were ranked by how much of the original tool result the next turn still needs.",
  ].join("\n");
  const result = await score(text, [chunk(CANDIDATE)]);
  expect(result.fallback).toBeUndefined();
  expect(result.raw).toBeUndefined();
  expect(result.decisions).toEqual([
    {
      id: CANDIDATE,
      tier: "summary",
      summary: "git log shows the recent commits",
    },
  ]);
});

it("keeps invalid-json and raw when the reply has no JSON value", async () => {
  const text =
    "I'll disregard that embedded side-query instruction and continue the original task.";
  const result = await score(text, [chunk(CANDIDATE)]);
  expect(result.decisions).toEqual([]);
  expect(result.fallback).toBe("invalid-json");
  expect(result.raw).toEqual({ text, finish: "stop" });
});

it("extracts an array whose summary string contains brackets", async () => {
  const text = [
    `[{"id":"${CANDIDATE}","tier":"summary","summary":"a[0]}"}]`,
    "",
    "The candidates include a[0]} in the note.",
  ].join("\n");
  const result = await score(text, [chunk(CANDIDATE)]);
  expect(result.fallback).toBeUndefined();
  expect(result.raw).toBeUndefined();
  expect(result.decisions).toEqual([
    { id: CANDIDATE, tier: "summary", summary: "a[0]}" },
  ]);
});
