/**
 * Reads tool arguments from an unknown JSON value.
 *
 * Contract: schemas are JSON Schema object literals on each tool. This module
 * only picks fields; it does not log.
 *
 * Intentionally not in the Log: rejected inputs. The isError string returned
 * by the tool is what the session logs.
 */

/** A tool result as tools build it: text for the model and the error flag. */
export type ToolResult = { content: string; isError: boolean };

/** A successful result with `content`. */
export function ok(content: string): ToolResult {
  return { content, isError: false };
}

/** An isError result with `content`; tools return it instead of throwing. */
export function fail(content: string): ToolResult {
  return { content, isError: true };
}

/** `input` as an object, or undefined for null, arrays, and primitives. */
export function asRecord(input: unknown): Record<string, unknown> | undefined {
  if (typeof input !== "object" || input === null || Array.isArray(input)) {
    return undefined;
  }
  return input as Record<string, unknown>;
}

/** `record[key]` when it is a string (possibly empty), otherwise undefined. */
export function requiredString(
  record: Record<string, unknown>,
  key: string,
): string | undefined {
  const value = record[key];
  return typeof value === "string" ? value : undefined;
}

/**
 * Reads an optional string field. Absent is `{ ok: true }` without a
 * value; present but not a string is `{ ok: false }`.
 */
export function optionalString(
  record: Record<string, unknown>,
  key: string,
): { ok: true; value?: string } | { ok: false } {
  const value = record[key];
  if (value === undefined) return { ok: true };
  if (typeof value !== "string") return { ok: false };
  return { ok: true, value };
}

/**
 * Reads an optional integer field. Absent is `{ ok: true }` without a
 * value; present but not an integer is `{ ok: false }`.
 */
export function optionalInt(
  record: Record<string, unknown>,
  key: string,
): { ok: true; value?: number } | { ok: false } {
  if (record[key] === undefined) return { ok: true };
  const value = record[key];
  if (typeof value !== "number" || !Number.isInteger(value))
    return { ok: false };
  return { ok: true, value };
}

/** The message of an Error, or `String(error)` for anything else. */
export function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
