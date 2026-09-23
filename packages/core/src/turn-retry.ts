/**
 * Whole-turn retry: resends one model request after a transport failure
 * that broke the stream after the provider's own pre-stream retries.
 *
 * Guarantees: a request is resent only while the turn has not appended an
 * assistant message, so no tool has run and no partial text is in the Log.
 * HTTP status errors are never resent here; the provider already retried
 * 429 and 5xx before the body began. Each resend is preceded by one
 * `turn_retry` entry.
 *
 * Intentionally not in the Log: the sleep function and the partial
 * assistant text of a retried call. `turn_retry` is persisted and is not
 * model-visible.
 */

import {
  type AssistantMessage,
  backoffMs,
  collect,
  isRecord,
  type PreparedCall,
  type ProviderAdapter,
  type Request,
  type Sleeper,
} from "@sage/provider/contract";
import { lastHeaderIndex } from "./log/path.js";
import type { LogStore } from "./log/store.js";
import type { StoredEntry } from "./log/types.js";

/** Whole-turn resends after a transport error, on top of the first call. */
const TURN_RETRY_LIMIT = 2;
/** First whole-turn retry delay; the shared backoff curve makes the second 8s. */
const TURN_RETRY_BASE_MS = 2000;

/**
 * The failure message when the finish is a transport error, else undefined.
 * The provider sets `transport` for fetch and body-read failures and for a
 * body that ends before its stop; the message text is never inspected.
 */
function turnTransportReason(
  finish: AssistantMessage["finish"],
): string | undefined {
  if (finish.kind !== "error" || !finish.error) return undefined;
  return finish.error.transport === true ? finish.error.message : undefined;
}

function assistantAfterLatestHeader(path: readonly StoredEntry[]): boolean {
  const header = lastHeaderIndex(path);
  if (header < 0) return false;
  return path
    .slice(header + 1)
    .some(
      (entry) =>
        entry.kind === "message" &&
        isRecord(entry.payload) &&
        entry.payload.role === "assistant",
    );
}

/**
 * Streams `request` and collects it. On a transport error, while the turn
 * has not appended an assistant message, waits 2s then 8s and resends the
 * same request, at most `TURN_RETRY_LIMIT` times, appending
 * `turn_retry {turn, attempt, reason}` before each resend. Returns
 * "aborted" when the signal fires during a wait (no `turn_retry` is written
 * then); otherwise the last collected message, which may still be a
 * transport error after the last resend.
 */
export async function collectWithTurnRetry(call: {
  store: LogStore;
  sessionId: string;
  adapter: ProviderAdapter;
  sleep: Sleeper;
  turn: number;
  request: Request;
  prepared: PreparedCall;
  signal: AbortSignal | undefined;
}): Promise<AssistantMessage | "aborted"> {
  const { store, sessionId, adapter, signal } = call;
  let attempt = 0;
  while (true) {
    const assistant = await collect(
      adapter.stream(call.request, call.prepared, signal),
      { api: adapter.api },
    );
    if (assistant.finish.kind === "aborted" || signal?.aborted) {
      return assistant;
    }
    const reason = turnTransportReason(assistant.finish);
    const committed = assistantAfterLatestHeader(store.path(sessionId));
    if (reason === undefined || committed || attempt >= TURN_RETRY_LIMIT) {
      return assistant;
    }
    attempt += 1;
    await call.sleep(backoffMs(attempt, TURN_RETRY_BASE_MS), signal);
    if (signal?.aborted) return "aborted";
    store.append(sessionId, {
      kind: "turn_retry",
      payload: { turn: call.turn, attempt, reason },
    });
  }
}
