import { describe, expect, it } from "vitest";
import type {
  ModelRef,
  ProviderAdapter,
  Request,
  StreamChunk,
} from "../src/contract/index.ts";
import { drain, fixtureFetch, stripRaw } from "./support.ts";

export const SHARED_HAPPY: StreamChunk[] = [
  { type: "block_start", index: 0, blockType: "text" },
  { type: "text_delta", index: 0, text: "Hi" },
  { type: "block_end", index: 0 },
  {
    type: "block_start",
    index: 1,
    blockType: "tool_use",
    id: "toolu_1",
    name: "lookup",
  },
  { type: "tool_input_delta", index: 1, partialJson: '{"q":"x"}' },
  { type: "block_end", index: 1 },
  {
    type: "usage",
    usage: {
      inputOther: 10,
      inputCacheRead: 2,
      inputCacheCreation: 0,
      output: 12,
    },
  },
  { type: "finish", kind: "tool_use" },
];

const model: ModelRef = {
  provider: "anthropic",
  model: "claude-sonnet-4-5",
};

function request(): Request {
  return {
    model,
    head: { system: "sys", tools: [] },
    messages: [{ role: "user", content: [{ type: "text", text: "hi" }] }],
    cacheBreakpoints: [],
    sampling: { maxTokens: 16 },
  };
}

export function expectStreamInvariants(chunks: StreamChunk[]): void {
  const finishAt = chunks.findIndex((chunk) => chunk.type === "finish");
  expect(finishAt).toBe(chunks.length - 1);
  expect(chunks.filter((chunk) => chunk.type === "finish")).toHaveLength(1);
  const responses = chunks.filter((chunk) => chunk.type === "response");
  expect(responses.length).toBeLessThanOrEqual(1);
  if (responses.length === 1) {
    const responseAt = chunks.findIndex((chunk) => chunk.type === "response");
    const firstBlockAt = chunks.findIndex(
      (chunk) => chunk.type === "block_start",
    );
    if (firstBlockAt >= 0) expect(responseAt).toBeLessThan(firstBlockAt);
  }
  const usageAt = chunks.findIndex((chunk) => chunk.type === "usage");
  expect(usageAt).toBeGreaterThanOrEqual(0);
  expect(usageAt).toBeLessThan(finishAt);

  const phase = new Map<number, "open" | "closed">();
  for (const chunk of chunks) {
    if (chunk.type === "block_start") {
      expect(phase.has(chunk.index)).toBe(false);
      phase.set(chunk.index, "open");
      continue;
    }
    if (
      chunk.type === "text_delta" ||
      chunk.type === "thinking_delta" ||
      chunk.type === "tool_input_delta"
    ) {
      expect(phase.get(chunk.index)).toBe("open");
      continue;
    }
    if (chunk.type === "block_end") {
      expect(phase.get(chunk.index)).toBe("open");
      phase.set(chunk.index, "closed");
    }
  }
}

export function runContractSuite(
  name: string,
  makeAdapter: (fetchImpl: typeof fetch) => ProviderAdapter,
  fixtures: {
    happy: string;
    truncated: string;
    response: Extract<StreamChunk, { type: "response" }>;
  },
): void {
  describe(name, () => {
    it("scenario 1: drops raw and matches the shared transcript", async () => {
      const { fetchImpl } = fixtureFetch(fixtures.happy);
      const adapter = makeAdapter(fetchImpl);
      const chunks = await drain(
        adapter.stream(request(), adapter.prepare(model)),
      );
      const usage = chunks.find((chunk) => chunk.type === "usage");
      expect(usage?.type).toBe("usage");
      if (usage?.type === "usage") expect(usage.usage.raw).toBeDefined();
      expect(stripRaw(chunks)).toEqual([fixtures.response, ...SHARED_HAPPY]);
    });

    it("scenario 2: usage precedes finish, and blocks stay framed", async () => {
      const { fetchImpl } = fixtureFetch(fixtures.happy);
      const adapter = makeAdapter(fetchImpl);
      const chunks = await drain(
        adapter.stream(request(), adapter.prepare(model)),
      );
      expectStreamInvariants(chunks);
    });

    it("scenario 4: a cut stream finishes with error and keeps deltas", async () => {
      const { fetchImpl } = fixtureFetch(fixtures.truncated);
      const adapter = makeAdapter(fetchImpl);
      const chunks = await drain(
        adapter.stream(request(), adapter.prepare(model)),
      );
      expect(
        chunks
          .filter((chunk) => chunk.type === "text_delta")
          .map((chunk) => {
            if (chunk.type !== "text_delta") return "";
            return chunk.text;
          }),
      ).toEqual(["Hi"]);
      expect(chunks.some((chunk) => chunk.type === "tool_input_delta")).toBe(
        false,
      );
      expect(chunks.at(-1)).toMatchObject({
        type: "finish",
        kind: "error",
        error: { message: "stream ended before completion" },
      });
      expectStreamInvariants(chunks);
    });

    it("scenario 5: abort after the response and two chunks finishes aborted", async () => {
      const { fetchImpl } = fixtureFetch(fixtures.happy);
      const adapter = makeAdapter(fetchImpl);
      const controller = new AbortController();
      const chunks: StreamChunk[] = [];
      for await (const chunk of adapter.stream(
        request(),
        adapter.prepare(model),
        controller.signal,
      )) {
        chunks.push(chunk);
        if (chunks.length === 3) controller.abort();
      }
      expect(chunks[0]).toEqual(fixtures.response);
      expect(chunks[1]).toEqual(SHARED_HAPPY[0]);
      expect(chunks[2]).toEqual(SHARED_HAPPY[1]);
      expect(chunks.some((chunk) => chunk.type === "tool_input_delta")).toBe(
        false,
      );
      expect(chunks.at(-1)).toEqual({ type: "finish", kind: "aborted" });
      expectStreamInvariants(chunks);
    });
  });
}
