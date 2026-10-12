/**
 * The one `…` truncation the plugins use: stub input summaries and the
 * text the jev scorer puts into its state.
 *
 * Guarantees: a clipped string is at most `limit` UTF-16 code units long
 * and ends (or starts) with `…` when anything was cut. A surrogate pair is
 * never split, so a cut never leaves half an emoji.
 *
 * Intentionally not in the Log: these helpers. Stubs they shape are
 * model-visible and rebuilt from the Log by the same function.
 */

const ELLIPSIS = "…";

/** Keeps the start of `text`, at most `limit` characters including `…`. */
export function clipEnd(text: string, limit: number): string {
  if (text.length <= limit) return text;
  if (limit <= 0) return "";
  return `${wholeSlice(text, 0, limit - ELLIPSIS.length)}${ELLIPSIS}`;
}

/** Keeps the end of `text`, at most `limit` characters including `…`. */
export function clipStart(text: string, limit: number): string {
  if (text.length <= limit) return text;
  if (limit <= 0) return "";
  const keep = limit - ELLIPSIS.length;
  return `${ELLIPSIS}${wholeSlice(text, text.length - keep, text.length)}`;
}

/** `text.slice(start, end)` narrowed so neither edge splits a surrogate pair. */
function wholeSlice(text: string, start: number, end: number): string {
  let from = start;
  let to = end;
  if (from > 0 && isLowSurrogate(text.charCodeAt(from))) from += 1;
  if (to < text.length && isLowSurrogate(text.charCodeAt(to))) to -= 1;
  return text.slice(from, Math.max(from, to));
}

function isLowSurrogate(code: number): boolean {
  return code >= 0xdc00 && code <= 0xdfff;
}
