/**
 * One-line stub for an omitted chunk.
 *
 * Contract: `renderStub` returns
 * `[#<id> · <toolName> · <summary> · <n>k tokens · omitted · expand("<id>") to retrieve]`.
 * `<n>k` is `chunk.tokens / 1000` with one decimal. `summary` is
 * `summarizeInput(chunk.input)`: one line, at most 80 characters, cut with `…`.
 *
 * Intentionally not in the Log: this function. The stub is model-visible.
 * Rebuild reproduces it from `originalTokens` and the logged tool_use.
 */

import type { Chunk } from "./chunks.js";

const SUMMARY_LIMIT = 80;

export function summarizeInput(input: unknown): string {
  const line = compact(input);
  if (line.length <= SUMMARY_LIMIT) return line;
  return `${line.slice(0, SUMMARY_LIMIT - 1)}…`;
}

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
