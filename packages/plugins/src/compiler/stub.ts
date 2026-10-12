/**
 * One-line stub for an omitted chunk.
 *
 * Contract: `renderStub` returns
 * `[#<id> · <toolName> · <summary> · <n>k tokens · omitted · expand("<id>") to retrieve]`.
 * `<n>k` is `chunk.tokens / 1000` with one decimal. `summary` is
 * `summarizeInput(chunk.input)`: one line, at most `INPUT_LINE_LIMIT`
 * characters, cut with `…` by `clipEnd` (never inside a surrogate pair).
 *
 * Intentionally not in the Log: this function. The stub is model-visible.
 * Rebuild reproduces it from `originalTokens` and the logged tool_use.
 */

import type { Chunk } from "./chunks.js";
import { clipEnd } from "./clip.js";

/** Maximum characters of the one-line tool input shown in a stub. */
const INPUT_LINE_LIMIT = 80;

/**
 * One-line summary of a tool input: a string with whitespace collapsed, or
 * compact JSON, at most `limit` characters (default `INPUT_LINE_LIMIT`),
 * cut with `…` by `clipEnd`.
 */
export function summarizeInput(
  input: unknown,
  limit = INPUT_LINE_LIMIT,
): string {
  return clipEnd(compact(input), limit);
}

/**
 * A stub is the one line left in the view when a chunk is omitted: what
 * it was, how large, and how to get it back with `expand`.
 */
export function renderStub(chunk: Chunk): string {
  const name = chunk.toolName ?? "";
  const summary = summarizeInput(chunk.input);
  const size = `${(chunk.tokens / 1000).toFixed(1)}k`;
  return `[#${chunk.id} · ${name} · ${summary} · ${size} tokens · omitted · expand("${chunk.id}") to retrieve]`;
}

function compact(input: unknown): string {
  if (typeof input === "string") return input.replace(/\s+/g, " ").trim();
  if (typeof input === "number" || typeof input === "boolean") {
    return String(input);
  }
  if (input === undefined || input === null) return "";
  try {
    const json = JSON.stringify(input);
    if (typeof json !== "string") return "";
    return json.replace(/\s+/g, " ").trim();
  } catch {
    return "";
  }
}
