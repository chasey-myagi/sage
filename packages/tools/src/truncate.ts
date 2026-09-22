/**
 * Line and byte caps for tool text.
 *
 * Contract: never return a partial line. When both limits fit, the input is
 * returned unchanged. Otherwise the result is a prefix of whole lines.
 *
 * Intentionally not in the Log: this function does not append. Callers put
 * the returned text into a tool result, which the session logs.
 */

export type Truncation = {
  text: string;
  omitted: { lines: number; bytes: number };
};

export function splitLines(input: string): string[] {
  if (input.length === 0) return [];
  const parts = input.split("\n");
  if (input.endsWith("\n")) parts.pop();
  return parts;
}

export function truncate(
  input: string,
  limits: { maxLines: number; maxBytes: number },
): Truncation {
  const lines = splitLines(input);
  const totalBytes = Buffer.byteLength(input, "utf8");
  if (lines.length <= limits.maxLines && totalBytes <= limits.maxBytes) {
    return { text: input, omitted: { lines: 0, bytes: 0 } };
  }
  const kept: string[] = [];
  let used = 0;
  for (const line of lines) {
    if (kept.length >= limits.maxLines) break;
    const extra = Buffer.byteLength(line, "utf8") + (kept.length > 0 ? 1 : 0);
    if (used + extra > limits.maxBytes) break;
    kept.push(line);
    used += extra;
  }
  const text = kept.join("\n");
  return {
    text,
    omitted: {
      lines: lines.length - kept.length,
      bytes: totalBytes - Buffer.byteLength(text, "utf8"),
    },
  };
}
