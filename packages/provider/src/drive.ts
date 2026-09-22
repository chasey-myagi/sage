/**
 * Read one SSE response and let the adapter translate events.
 * Every exit yields usage and then a single finish chunk.
 *
 * Deliberately not a Log entry: this is the live decode of one call.
 * Wire frames stay in the SSE parser. The abort signal stays here.
 */

import type { ContentBlock } from "./contract/message.js";
import type { FinishKind, StreamChunk } from "./contract/stream.js";
import type { Usage } from "./contract/usage.js";
import { isAbortError, messageOf, postJson } from "./http.js";
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
  onEvent(event: SseEvent, driver: Driver): void;
}): AsyncGenerator<StreamChunk> {
  let usage: Usage | undefined;
  let stop: WireStop | undefined;
  let sealed = false;
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

  try {
    if (args.signal?.aborted) {
      sealed = true;
      yield* end(usage, { type: "finish", kind: "aborted" });
      return;
    }
    const opened = await postJson({
      url: args.url,
      headers: args.headers,
      json: args.json,
      signal: args.signal,
      fetchImpl: args.fetchImpl,
    });
    if (!opened.ok) {
      sealed = true;
      if (opened.aborted) {
        yield* end(usage, { type: "finish", kind: "aborted" });
      } else {
        yield* end(
          usage,
          errorFinish(opened.message, opened.status, opened.raw),
        );
      }
      return;
    }
    for await (const event of parseSse(opened.body)) {
      if (args.signal?.aborted) {
        sealed = true;
        yield* end(usage, { type: "finish", kind: "aborted" });
        return;
      }
      try {
        args.onEvent(event, driver);
      } catch (error) {
        yield* pending.splice(0, pending.length);
        sealed = true;
        yield* end(usage, failureFinish(args.signal, error));
        return;
      }
      yield* take(pending, args.signal);
      if (args.signal?.aborted) {
        sealed = true;
        yield* end(usage, { type: "finish", kind: "aborted" });
        return;
      }
      if (stop) {
        sealed = true;
        yield* end(usage, successFinish(stop));
        return;
      }
    }
    if (args.signal?.aborted) {
      sealed = true;
      yield* end(usage, { type: "finish", kind: "aborted" });
      return;
    }
    sealed = true;
    yield* end(usage, errorFinish("stream ended before completion"));
  } catch (error) {
    if (sealed) return;
    yield* end(usage, failureFinish(args.signal, error));
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
): StreamChunk {
  if (signal?.aborted || isAbortError(error)) {
    return { type: "finish", kind: "aborted" };
  }
  return errorFinish(messageOf(error));
}

function* end(
  usage: Usage | undefined,
  finish: StreamChunk,
): Generator<StreamChunk> {
  yield { type: "usage", usage: usage ?? zeroUsage() };
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

function errorFinish(
  message: string,
  status?: number,
  raw?: unknown,
): StreamChunk {
  const error: { message: string; status?: number; raw?: unknown } = {
    message,
  };
  if (status !== undefined) error.status = status;
  if (raw !== undefined) error.raw = raw;
  return { type: "finish", kind: "error", error };
}

function zeroUsage(): Usage {
  return {
    inputOther: 0,
    inputCacheRead: 0,
    inputCacheCreation: 0,
    output: 0,
  };
}
