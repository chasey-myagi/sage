/**
 * Spill oversized tool output to a file and return a head/tail preview.
 *
 * Contract: when output exceeds SPILL_LIMIT_BYTES, the full text is written
 * to the given path and the returned preview keeps a 20000 byte head and a
 * 5000 byte tail. The notice line is reserved inside the limit.
 *
 * Intentionally not in the Log: the spilled file. The preview, including the
 * path in the notice, is what the model sees and what the session logs.
 */

import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";

export const SPILL_LIMIT_BYTES = 30_000;
export const SPILL_HEAD_BYTES = 20_000;
export const SPILL_TAIL_BYTES = 5_000;

export function spill(
  output: string,
  filePath: string,
): { text: string; saved: boolean } {
  if (Buffer.byteLength(output, "utf8") <= SPILL_LIMIT_BYTES) {
    return { text: output, saved: false };
  }
  mkdirSync(path.dirname(filePath), { recursive: true });
  writeFileSync(filePath, output, "utf8");
  return { text: fit(output, filePath), saved: true };
}

function notice(omitted: number, filePath: string): string {
  return `[… omitted ${omitted} bytes; full output saved to ${filePath}; read it with read(path, offset=…) …]`;
}

function cutPrefix(text: string, maxBytes: number): string {
  const buf = Buffer.from(text, "utf8");
  if (buf.length <= maxBytes) return text;
  let end = maxBytes;
  while (end > 0 && ((buf[end] ?? 0) & 0xc0) === 0x80) end -= 1;
  return buf.subarray(0, end).toString("utf8");
}

function cutSuffix(text: string, maxBytes: number): string {
  const buf = Buffer.from(text, "utf8");
  if (buf.length <= maxBytes) return text;
  let start = buf.length - maxBytes;
  while (start < buf.length && ((buf[start] ?? 0) & 0xc0) === 0x80) {
    start += 1;
  }
  return buf.subarray(start).toString("utf8");
}

function fit(output: string, filePath: string): string {
  const total = Buffer.byteLength(output, "utf8");
  let headBudget = Math.min(SPILL_HEAD_BYTES, total);
  let tailBudget = Math.min(SPILL_TAIL_BYTES, total);
  let body = output;
  for (let attempt = 0; attempt < 8; attempt += 1) {
    if (headBudget + tailBudget > total) {
      tailBudget = Math.max(0, total - headBudget);
    }
    const head = cutPrefix(output, headBudget);
    const tail = cutSuffix(output, tailBudget);
    const headBytes = Buffer.byteLength(head, "utf8");
    const tailBytes = Buffer.byteLength(tail, "utf8");
    const line = notice(total - headBytes - tailBytes, filePath);
    body = `${head}\n${line}\n${tail}`;
    const size = Buffer.byteLength(body, "utf8");
    if (size <= SPILL_LIMIT_BYTES) return body;
    const over = size - SPILL_LIMIT_BYTES;
    if (tailBudget > 0) tailBudget = Math.max(0, tailBudget - over);
    else headBudget = Math.max(0, headBudget - over);
  }
  return body;
}
