import { describe, expect, it } from "vitest";
import type {
  ModelRef,
  ProviderAdapter,
  Request,
  StreamChunk,
} from "../src/contract/index.ts";
import {
  drain,
  type FetchStep,
  fixtureFetch,
  scriptedFetch,
  stripRaw,
} from "./support.ts";

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

type FailureCase = {
  name: string;
  steps: (fixtures: { truncated: string; errorEvent: string }) => FetchStep[];
  /** The failure happened before the response body began. */
  preBody: boolean;
  cause: "transport" | "http" | "event";
  status?: number;
  /** The finish error message, when the server supplied it. */
  message?: string;
};

const ERROR_BODY = '{"error":{"message":"upstream said no"}}';

/** finish.error.cause for each way a call can fail. */
const FAILURES: FailureCase[] = [
  {
    name: "fetch throws on every attempt",
    steps: () => [{ action: "throw", message: "getaddrinfo EAI_AGAIN" }],
    preBody: true,
    cause: "transport",
  },
  {
    name: "reading a non-2xx error body throws",
    steps: () => [
      { action: "cut", status: 502, body: "upst", message: "terminated" },
    ],
    preBody: true,
    cause: "transport",
  },
  {
    name: "a 2xx response has no body",
    steps: () => [{ action: "respond", status: 200, body: null }],
    preBody: true,
    cause: "transport",
  },
  {
    name: "HTTP 500 outlasts the pre-stream retries",
    steps: () => [{ action: "respond", status: 500, body: ERROR_BODY }],
    preBody: true,
    cause: "http",
    status: 500,
  },
  {
    name: "HTTP 400 is read and not retried",
    steps: () => [{ action: "respond", status: 400, body: ERROR_BODY }],
    preBody: true,
    cause: "http",
    status: 400,
  },
  {
    name: "the body read throws mid-stream",
    steps: ({ truncated }) => [
      {
        action: "cut",
        status: 200,
        body: truncated,
        message: "other side closed",
      },
    ],
    preBody: false,
    cause: "transport",
  },
  {
    name: "the body closes before the stream completes",
    steps: ({ truncated }) => [
      { action: "respond", status: 200, body: truncated },
    ],
    preBody: false,
    cause: "transport",
  },
  {
    name: "the stream carries an error event",
    steps: ({ errorEvent }) => [
      { action: "respond", status: 200, body: errorEvent },
    ],
    preBody: false,
    cause: "event",
    message: "Overloaded",
  },
];

async function noSleep(): Promise<void> {}

/** A fetch that never settles on its own and rejects once `signal` fires. */
function hangingFetch(): typeof fetch {
  return (_input, init) =>
    new Promise<Response>((_resolve, reject) => {
      const signal = init?.signal;
      signal?.addEventListener("abort", () => {
        reject(new DOMException("The operation was aborted.", "AbortError"));
      });
    });
}

export function runContractSuite(
  name: string,
  makeAdapter: (
    fetchImpl: typeof fetch,
    sleep?: (ms: number) => Promise<void>,
  ) => ProviderAdapter,
  fixtures: {
    happy: string;
    truncated: string;
    /** Streams `Hi`, then a server error event "Overloaded", then more text. */
    errorEvent: string;
    /** `finish.error.event` decoded from `errorEvent`. */
    errorEventClass: Record<string, unknown>;
    response: Extract<StreamChunk, { type: "response" }>;
  },
): void {
  describe(name, () => {
    it("drops raw and matches the shared transcript", async () => {
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

    it("emits usage before finish and keeps blocks framed", async () => {
      const { fetchImpl } = fixtureFetch(fixtures.happy);
      const adapter = makeAdapter(fetchImpl);
      const chunks = await drain(
        adapter.stream(request(), adapter.prepare(model)),
      );
      expectStreamInvariants(chunks);
    });

    it("finishes a cut stream with error and keeps its deltas", async () => {
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

    it("stops at an in-stream error event and keeps the deltas before it", async () => {
      const { fetchImpl } = fixtureFetch(fixtures.errorEvent);
      const adapter = makeAdapter(fetchImpl);
      const chunks = await drain(
        adapter.stream(request(), adapter.prepare(model)),
      );
      expect(
        chunks
          .filter((chunk) => chunk.type === "text_delta")
          .map((chunk) => (chunk.type === "text_delta" ? chunk.text : "")),
      ).toEqual(["Hi"]);
      const finish = chunks.at(-1);
      expect(finish).toMatchObject({
        type: "finish",
        kind: "error",
        error: { message: "Overloaded" },
      });
      if (finish?.type === "finish") {
        expect(finish.error?.raw).toMatchObject({
          error: { message: "Overloaded" },
        });
        expect(finish.error?.cause).toBe("event");
        if (finish.error?.cause === "event") {
          expect(finish.error.event).toEqual(fixtures.errorEventClass);
        }
      }
      expectStreamInvariants(chunks);
    });

    it("finishes aborted when the signal fires after the response and two chunks", async () => {
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

    for (const failure of FAILURES.filter((item) => item.preBody)) {
      it(`has no response chunk when ${failure.name}`, async () => {
        const { fetchImpl } = scriptedFetch(failure.steps(fixtures));
        const adapter = makeAdapter(fetchImpl, noSleep);
        const chunks = await drain(
          adapter.stream(request(), adapter.prepare(model)),
        );
        expect(chunks.some((chunk) => chunk.type === "response")).toBe(false);
        expect(chunks.at(-1)).toMatchObject({ type: "finish", kind: "error" });
        expectStreamInvariants(chunks);
      });
    }

    for (const failure of FAILURES) {
      it(`finishes with cause ${failure.cause} when ${failure.name}`, async () => {
        const { fetchImpl } = scriptedFetch(failure.steps(fixtures));
        const adapter = makeAdapter(fetchImpl, noSleep);
        const chunks = await drain(
          adapter.stream(request(), adapter.prepare(model)),
        );
        const finish = chunks.at(-1);
        if (finish?.type !== "finish" || !finish.error) {
          throw new Error(`expected an error finish, got ${String(finish)}`);
        }
        expect(finish.kind).toBe("error");
        expect(finish.error.cause).toBe(failure.cause);
        expect(
          finish.error.cause === "http" ? finish.error.status : undefined,
        ).toBe(failure.status);
        expect(Object.hasOwn(finish.error, "status")).toBe(
          failure.cause === "http",
        );
        expect(Object.hasOwn(finish.error, "event")).toBe(
          failure.cause === "event",
        );
        if (failure.message !== undefined) {
          expect(finish.error.message).toBe(failure.message);
        }
        expectStreamInvariants(chunks);
      });
    }

    it("finishes aborted, not error, when the signal fired before the call", async () => {
      const { fetchImpl, calls } = scriptedFetch([
        { action: "throw", message: "unreachable" },
      ]);
      const adapter = makeAdapter(fetchImpl, noSleep);
      const controller = new AbortController();
      controller.abort();
      const chunks = await drain(
        adapter.stream(request(), adapter.prepare(model), controller.signal),
      );
      expect(chunks.at(-1)).toEqual({ type: "finish", kind: "aborted" });
      expect(calls()).toBe(0);
      expectStreamInvariants(chunks);
    });

    it("finishes aborted, not error, when the signal fires during fetch", async () => {
      const adapter = makeAdapter(hangingFetch(), noSleep);
      const controller = new AbortController();
      const pending = drain(
        adapter.stream(request(), adapter.prepare(model), controller.signal),
      );
      controller.abort();
      const chunks = await pending;
      expect(chunks.at(-1)).toEqual({ type: "finish", kind: "aborted" });
      expect(chunks.some((chunk) => chunk.type === "response")).toBe(false);
      expectStreamInvariants(chunks);
    });
  });
}
