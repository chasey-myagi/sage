/**
 * How the child's session ended, read off its copied Log path.
 *
 * Contract: pure functions of one session path. They read the terminal
 * entry's `outcome` and the last assistant message's `details.finish`;
 * message text, tool output, and the error message are never read, so a
 * provider's error text cannot reach `results.json`.
 *
 * Intentionally not in the Log: the fault. It lives in `results.json`.
 */

import {
  isFiniteNumber,
  isMessage,
  isRecord,
  type StoredEntry,
} from "@sage/core";
import type {
  BenchErrorReason,
  BenchProviderError,
  BenchStreamErrorEvent,
} from "./types.js";

/**
 * A run that did not finish on its own terms. `session-error` is a terminal
 * `error` outcome; `session-aborted` is a terminal `aborted` outcome.
 */
export type SessionFault =
  | {
      reason: Extract<BenchErrorReason, "session-error">;
      providerError: BenchProviderError | null;
    }
  | { reason: Extract<BenchErrorReason, "session-aborted"> };

/** `outcome` of the path's last entry when it is a terminal, else undefined. */
export function terminalOutcome(
  path: readonly StoredEntry[],
): string | undefined {
  const last = path.at(-1);
  if (last?.kind !== "terminal" || !isRecord(last.payload)) return undefined;
  return typeof last.payload.outcome === "string"
    ? last.payload.outcome
    : undefined;
}

/**
 * The fault that ended the run, or null for `done`, `max_turns`, any other
 * outcome, or a path without a terminal entry.
 */
export function sessionFault(
  path: readonly StoredEntry[],
): SessionFault | null {
  switch (terminalOutcome(path)) {
    case "error":
      return { reason: "session-error", providerError: providerError(path) };
    case "aborted":
      return { reason: "session-aborted" };
    default:
      return null;
  }
}

/**
 * The classification of the last assistant message's finish error, or null
 * when that message did not end with `finish.kind === "error"` or there is
 * no assistant message. A run ends on the first error finish, so the last
 * assistant message is the one that ended it.
 */
function providerError(
  path: readonly StoredEntry[],
): BenchProviderError | null {
  const finish = lastAssistant(path)?.details?.finish;
  if (!isRecord(finish) || finish.kind !== "error") return null;
  const error = isRecord(finish.error) ? finish.error : {};
  return {
    status: isFiniteNumber(error.status) ? error.status : null,
    transport: error.transport === true,
    event: streamErrorEvent(error.event),
  };
}

/** `FinishError.event` with absent fields as null, or null when absent. */
function streamErrorEvent(value: unknown): BenchStreamErrorEvent | null {
  if (!isRecord(value) || typeof value.transient !== "boolean") return null;
  const { code } = value;
  return {
    transient: value.transient,
    type: typeof value.type === "string" ? value.type : null,
    code: typeof code === "string" || isFiniteNumber(code) ? code : null,
    status: isFiniteNumber(value.status) ? value.status : null,
  };
}

function lastAssistant(path: readonly StoredEntry[]): StoredEntry | undefined {
  for (let index = path.length - 1; index >= 0; index -= 1) {
    const entry = path[index];
    if (
      entry?.kind === "message" &&
      isMessage(entry.payload) &&
      entry.payload.role === "assistant"
    ) {
      return entry;
    }
  }
  return undefined;
}
