/**
 * Guards for wire JSON objects.
 *
 * Deliberately not a Log entry: these helpers allocate nothing that
 * a later request_header would store.
 */

import { isRecord } from "./contract/record.js";

export function asRecord(value: unknown): Record<string, unknown> | undefined {
  return isRecord(value) ? value : undefined;
}

export function num(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

export function indexOf(value: unknown): number | undefined {
  if (typeof value !== "number" || !Number.isInteger(value) || value < 0) {
    return undefined;
  }
  return value;
}
