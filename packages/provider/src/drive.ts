/**
 * Reads one SSE response and lets the adapter translate its events: the
 * protocol-independent half of every adapter.
 *
 * Guarantees: every exit yields usage and then a single finish chunk.
 * Fetch throws, HTTP 429, and HTTP 5xx are retried before the body starts;
 * once parsing begins, a later failure is not retried. A fetch throw, a
 * throw while reading the body, and a body that ends before the adapter
 * records a stop finish with `error.transport = true`; a throw from the
 * adapter's `onEvent` and an error event the adapter records do not.
 *
 * Intentionally not in the Log: this is the live decode of one call. Wire
 * frames stay in the SSE parser and the abort signal stays here.
 * Intermediate failure bodies are not entries. Retry count and the last
 * failure reason are copied onto the usage chunk.
 */

import type { ContentBlock } from "./contract/message.js";
import {
  type FinishError,
  type FinishKind,
  INCOMPLETE_STREAM,
  type StreamChunk,
} from "./contract/stream.js";
import { emptyUsage, type Usage } from "./contract/usage.js";
import {
  type Client,
  isAbortError,
  joinUrl,
  messageFromPayload,
  messageOf,
} from "./http.js";
import { type OpenedCall, openWithRetry } from "./retry.js";
import type { SseEvent } from "./sse.js";
import { parseSse } from "./sse.js";

/** The stop an adapter decoded from the wire; becomes the finish chunk. */
export type WireStop = {
  kind: FinishKind;
  rawFinishReason?: string;
};

/**
 * What `onEvent` may do: queue chunks, replace the running usage, and
 * record how the response ended: `complete` for a stop, `fail` for an
 * error event the server sent inside the stream. Only the first `complete`
 * or `fail` counts; the stream finishes after the event that called it.
 */
export type Driver = {
  push(chunk: StreamChunk): void;
  setUsage(usage: Usage): void;
  complete(stop: WireStop): void;
  /**
   * Records a server error event. `raw` is the parsed event object; the
   * finish message is its `error.message` (or `message`), else
   * `STREAM_ERROR_EVENT`. The finish is not a transport failure.
   */
  fail(raw: Record<string, unknown>): void;
};

/** Finish message for an in-stream error event that carries no message. */
const STREAM_ERROR_EVENT = "provider sent an error event";

/** A block_start chunk; `id` and `name` are omitted when undefined. */
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

/** One streamed POST: what differs between adapters. */
export type SseCall = {
  /** Appended to the client's base URL. */
  path: string;
  headers: Record<string, string>;
  json: unknown;
  signal?: AbortSignal;
  /** Translates one wire event; runs synchronously between yields. */
  onEvent(event: SseEvent, driver: Driver): void;
};

/**
 * Streams one call. Never throws; see the guarantees above. Aborting
 * `call.signal` finishes as `aborted` at the next chunk boundary.
 */
export async function* driveStream(
  client: Client,
  call: SseCall,
): AsyncGenerator<StreamChunk> {
  const signal = call.signal;
  const decode = startDecode();
  let opened: OpenedCall | undefined;
  let finish: StreamChunk;
  try {
    if (!signal?.aborted) {
      opened = await openWithRetry({
        url: joinUrl(client.baseUrl, call.path),
        headers: call.headers,
        json: call.json,
        signal,
        fetchImpl: client.fetchImpl,
        retry: client.retry,
        sleep: client.sleep,
      });
    }
    if (!opened) finish = abortedFinish();
    else if (!opened.ok) finish = openFailure(opened);
    else finish = yield* readBody(opened.body, call, decode);
  } catch (error) {
    // postJson already turned fetch and error-body throws into data, so
    // only the SSE body read can throw here: a failure below HTTP.
    finish = failureFinish(signal, error, opened?.ok === true);
  }
  const usage = withRetry(decode.usage, opened?.retries ?? 0, opened?.reason);
  yield* end(usage, finish);
}

/** How the response ended, as the adapter recorded it. */
type WireEnd =
  | { type: "stop"; stop: WireStop }
  | { type: "error"; raw: Record<string, unknown> };

/** Decoder state `onEvent` writes through its `Driver`. */
type Decode = {
  usage?: Usage;
  end?: WireEnd;
  pending: StreamChunk[];
  driver: Driver;
};

function startDecode(): Decode {
  const pending: StreamChunk[] = [];
  const decode: Decode = {
    pending,
    driver: {
      push(chunk) {
        pending.push(chunk);
      },
      setUsage(next) {
        decode.usage = next;
      },
      complete(stop) {
        decode.end ??= { type: "stop", stop };
      },
      fail(raw) {
        decode.end ??= { type: "error", raw };
      },
    },
  };
  return decode;
}

/**
 * Feeds SSE events to `onEvent` and yields the chunks it queues. Returns
 * the finish chunk: the recorded stop or error event, `aborted`, an `onEvent` failure
 * (after yielding what it queued first), or a transport `INCOMPLETE_STREAM`
 * when the body ends without a stop. A throw while reading the body
 * propagates.
 */
async function* readBody(
  body: ReadableStream<Uint8Array>,
  call: SseCall,
  decode: Decode,
): AsyncGenerator<StreamChunk, StreamChunk> {
  const signal = call.signal;
  for await (const event of parseSse(body)) {
    if (signal?.aborted) return abortedFinish();
    try {
      call.onEvent(event, decode.driver);
    } catch (error) {
      yield* decode.pending.splice(0, decode.pending.length);
      return failureFinish(signal, error, false);
    }
    yield* take(decode.pending, signal);
    if (signal?.aborted) return abortedFinish();
    if (decode.end) return endFinish(decode.end);
  }
  // The connection closed before the response completed: the same failure
  // as a proxy cutting the socket, only without a thrown error.
  return signal?.aborted
    ? abortedFinish()
    : errorFinish({ message: INCOMPLETE_STREAM, transport: true });
}

function openFailure(opened: Extract<OpenedCall, { ok: false }>): StreamChunk {
  if (opened.aborted) return abortedFinish();
  return errorFinish({
    message: opened.message,
    status: opened.status,
    raw: opened.raw,
    transport: opened.transport,
  });
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

function abortedFinish(): StreamChunk {
  return { type: "finish", kind: "aborted" };
}

function failureFinish(
  signal: AbortSignal | undefined,
  error: unknown,
  transport: boolean,
): StreamChunk {
  if (signal?.aborted || isAbortError(error)) return abortedFinish();
  return errorFinish({ message: messageOf(error), transport });
}

function* end(
  usage: Usage | undefined,
  finish: StreamChunk,
): Generator<StreamChunk> {
  yield { type: "usage", usage: usage ?? emptyUsage() };
  yield finish;
}

function endFinish(end: WireEnd): StreamChunk {
  if (end.type === "stop") return successFinish(end.stop);
  return errorFinish({
    message: messageFromPayload(end.raw) ?? STREAM_ERROR_EVENT,
    raw: end.raw,
  });
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
