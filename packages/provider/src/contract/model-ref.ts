/**
 * Readers for `ModelRef` values from untrusted input: JSON objects and
 * `provider/model` text.
 *
 * Contract: pure functions; they never throw.
 *
 * Intentionally not in the Log: nothing here appends.
 */

import type { ModelRef } from "./message.js";
import { isRecord } from "./record.js";

/** True for an object whose `provider` and `model` are strings. */
export function isModelRef(value: unknown): value is ModelRef {
  return (
    isRecord(value) &&
    typeof value.provider === "string" &&
    typeof value.model === "string"
  );
}

/**
 * Parses `provider/model`, split at the first `/`. Returns undefined when
 * there is no `/` or either side is empty.
 */
export function parseModelRef(text: string): ModelRef | undefined {
  const slash = text.indexOf("/");
  if (slash <= 0 || slash === text.length - 1) return undefined;
  return { provider: text.slice(0, slash), model: text.slice(slash + 1) };
}
