/**
 * Readers for untrusted wire JSON, used by the adapters. Each returns a
 * neutral value (undefined or 0) instead of throwing on bad input.
 *
 * Intentionally not in the Log: these helpers only read.
 */

import { isRecord } from "./contract/record.js";

/** `value` when it is a plain object, else undefined. */
export function asRecord(value: unknown): Record<string, unknown> | undefined {
  return isRecord(value) ? value : undefined;
}

/**
 * Parses `text` as JSON and returns it when it is an object. Malformed JSON
 * and non-objects return undefined; adapters skip such SSE events.
 */
export function parseRecord(text: string): Record<string, unknown> | undefined {
  try {
    return asRecord(JSON.parse(text));
  } catch {
    // Malformed JSON: the caller skips this event and the stream continues.
    return undefined;
  }
}

/** `value` when it is a finite number, else 0. */
export function num(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

/** `value` when it is a non-negative integer, else undefined. */
export function indexOf(value: unknown): number | undefined {
  if (typeof value !== "number" || !Number.isInteger(value) || value < 0) {
    return undefined;
  }
  return value;
}
