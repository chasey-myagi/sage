/**
 * Read one SSE response and let the adapter translate events.
 * Every exit yields usage and then a single finish chunk.
 * Fetch throws, HTTP 429, and HTTP 5xx are retried before the body
 * starts. Once parseSse is entered, a later failure is not retried.
 * A fetch throw, and a throw while reading the body, finish with
 * error.transport = true. A throw from the adapter's onEvent does not.
 *
 * Deliberately not a Log entry: this is the live decode of one call.
 * Wire frames stay in the SSE parser. The abort signal stays here.
 * Intermediate failure bodies are not entries. Retry count and the
 * last failure reason are copied onto the usage chunk.
 */

import type { Sleeper } from "./contract/backoff.js";
import type { ContentBlock } from "./contract/message.js";
import type {
  FinishError,
  FinishKind,
  StreamChunk,
} from "./contract/stream.js";
import { emptyUsage, type Usage } from "./contract/usage.js";
import { isAbortError, messageOf } from "./http.js";
import type { RetryPolicy } from "./retry.js";
import { openWithRetry } from "./retry.js";
import type { SseEvent } from "./sse.js";
import { parseSse } from "./sse.js";

export type WireStop = {
  kind: FinishKind;
  rawFinishReason?: string;
};

export type Driver = {
  push(chunk: StreamChunk): void;
  setUsage(usage: Usage): void;
  complete(stop: WireStop): void;
};

export function blockStart(
  index: number,
  blockType: ContentBlock["type"],
  id?: string,
  name?: string,
): StreamChunk {
  const chunk: Extract<StreamChunk, { type: "block_start" }> = {
    type: "block_start",
    index,
    blockType,
  };
  if (id !== undefined) chunk.id = id;
  if (name !== undefined) chunk.name = name;
  return chunk;
}

export async function* driveStream(args: {
  url: string;
  headers: Record<string, string>;
  json: unknown;
  signal?: AbortSignal;
  fetchImpl?: typeof fetch;
  retry?: RetryPolicy;
  sleep?: Sleeper;
  onEvent(event: SseEvent, driver: Driver): void;
}): AsyncGenerator<StreamChunk> {
  let usage: Usage | undefined;
  let stop: WireStop | undefined;
  let sealed = false;
  let reading = false;
  let retries = 0;
  let retryReason: string | undefined;
  const pending: StreamChunk[] = [];
  const driver: Driver = {
    push(chunk) {
      pending.push(chunk);
    },
    setUsage(next) {
      usage = next;
    },
    complete(next) {
      if (!stop) stop = next;
    },
  };
  const noted = (): Usage | undefined => withRetry(usage, retries, retryReason);

  try {
    if (args.signal?.aborted) {
      sealed = true;
      yield* end(noted(), {
        type: "finish",
        kind: "aborted",
      });
      return;
    }
    const opened = await openWithRetry({
      url: args.url,
      headers: args.headers,
      json: args.json,
      signal: args.signal,
      fetchImpl: args.fetchImpl,
      retry: args.retry,
      sleep: args.sleep,
    });
    retries = opened.retries;
    retryReason = opened.reason;
    if (!opened.ok) {
      sealed = true;
      if (opened.aborted) {
        yield* end(noted(), {
          type: "finish",
          kind: "aborted",
        });
      } else {
        yield* end(
          noted(),
          errorFinish({
            message: opened.message,
            status: opened.status,
            raw: opened.raw,
            transport: opened.transport,
          }),
        );
      }
      return;
    }
    reading = true;
    for await (const event of parseSse(opened.body)) {
      if (args.signal?.aborted) {
        sealed = true;
        yield* end(noted(), {
          type: "finish",
          kind: "aborted",
        });
        return;
      }
      try {
        args.onEvent(event, driver);
      } catch (error) {
        yield* pending.splice(0, pending.length);
        sealed = true;
        yield* end(noted(), failureFinish(args.signal, error, false));
        return;
      }
      yield* take(pending, args.signal);
      if (args.signal?.aborted) {
        sealed = true;
        yield* end(noted(), {
          type: "finish",
          kind: "aborted",
        });
        return;
      }
      if (stop) {
        sealed = true;
        yield* end(noted(), successFinish(stop));
        return;
      }
    }
    if (args.signal?.aborted) {
      sealed = true;
      yield* end(noted(), {
        type: "finish",
        kind: "aborted",
      });
      return;
    }
    sealed = true;
    yield* end(
      noted(),
      errorFinish({ message: "stream ended before completion" }),
    );
  } catch (error) {
    if (sealed) return;
    // Only the body read can throw here once reading is set; that is a
    // failure below HTTP, the same class as a fetch throw.
    yield* end(noted(), failureFinish(args.signal, error, reading));
  }
}

function* take(
  pending: StreamChunk[],
  signal: AbortSignal | undefined,
): Generator<StreamChunk> {
  const batch = pending.splice(0, pending.length);
  for (const chunk of batch) {
    yield chunk;
    if (signal?.aborted) return;
  }
}

function failureFinish(
  signal: AbortSignal | undefined,
  error: unknown,
  transport: boolean,
): StreamChunk {
  if (signal?.aborted || isAbortError(error)) {
    return { type: "finish", kind: "aborted" };
  }
  return errorFinish({ message: messageOf(error), transport });
}

function* end(
  usage: Usage | undefined,
  finish: StreamChunk,
): Generator<StreamChunk> {
  yield { type: "usage", usage: usage ?? emptyUsage() };
  yield finish;
}

function successFinish(stop: WireStop): StreamChunk {
  if (stop.rawFinishReason !== undefined) {
    return {
      type: "finish",
      kind: stop.kind,
      rawFinishReason: stop.rawFinishReason,
    };
  }
  return { type: "finish", kind: stop.kind };
}

function errorFinish(from: {
  message: string;
  status?: number;
  raw?: unknown;
  transport?: boolean;
}): StreamChunk {
  const error: FinishError = { message: from.message };
  if (from.status !== undefined) error.status = from.status;
  if (from.raw !== undefined) error.raw = from.raw;
  if (from.transport) error.transport = true;
  return { type: "finish", kind: "error", error };
}

function withRetry(
  usage: Usage | undefined,
  retries: number,
  reason: string | undefined,
): Usage | undefined {
  if (retries <= 0 || reason === undefined) return usage;
  return { ...(usage ?? emptyUsage()), details: { retries, reason } };
}
