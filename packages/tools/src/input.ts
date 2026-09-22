/**
 * Reads tool arguments from an unknown JSON value.
 *
 * Contract: schemas are JSON Schema object literals on each tool. This module
 * only picks fields; it does not log.
 *
 * Intentionally not in the Log: rejected inputs. The isError string returned
 * by the tool is what the session logs.
 */

export type ToolResult = { content: string; isError: boolean };

export function ok(content: string): ToolResult {
  return { content, isError: false };
}

export function fail(content: string): ToolResult {
  return { content, isError: true };
}

export function asRecord(input: unknown): Record<string, unknown> | undefined {
  if (typeof input !== "object" || input === null || Array.isArray(input)) {
    return undefined;
  }
  return input as Record<string, unknown>;
}

export function requiredString(
  record: Record<string, unknown>,
  key: string,
): string | undefined {
  const value = record[key];
  return typeof value === "string" ? value : undefined;
}

export function optionalString(
  record: Record<string, unknown>,
  key: string,
): { ok: true; value?: string } | { ok: false } {
  const value = record[key];
  if (value === undefined) return { ok: true };
  if (typeof value !== "string") return { ok: false };
  return { ok: true, value };
}

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

export function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
