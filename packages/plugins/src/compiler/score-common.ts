/**
 * Pieces shared by the three ways the compiler calls a scorer: on rebuild,
 * at entry, and in shadow.
 *
 * Contract: a scorer may only move a tool chunk to none, summary, or full;
 * any other tier in its reply is ignored. A summary note is cut to
 * `SUMMARY_LIMIT` characters. Usage copied onto the manifest goes through
 * `readTokenCounts`, so every count is a finite number.
 *
 * Intentionally not in the Log: these helpers and the request they build.
 * The scorer's decisions reach the Log only as manifest rows and reports.
 */

import type { CompiledView, ViewContext } from "@sage/core";
import type { Message, Request } from "@sage/provider/contract";
import type { ScoreResult } from "../scorer/types.js";
import type { Tier } from "./claim.js";
import { latestHeader } from "./log-read.js";

/** Tiers a scorer may assign. */
export const SCORE_TIERS: ReadonlySet<Tier> = new Set<Tier>([
  "none",
  "summary",
  "full",
]);

/** Maximum characters of a scorer's summary note kept on the chunk. */
export const SUMMARY_LIMIT = 200;

/** A summary note shorter than this many characters is not locatable. */
const SUMMARY_MIN_CHARS = 20;
/** Path, extension, digit, camelCase, or snake_case. */
const SUMMARY_ANCHOR =
  /(?:[/\\]|\.[A-Za-z0-9]{1,8}\b|\d|[A-Za-z][A-Za-z0-9]*[A-Z][A-Za-z0-9]*|[A-Za-z0-9]+_[A-Za-z0-9_]+)/;

/**
 * True when a note lets the model find the evidence again: at least
 * `SUMMARY_MIN_CHARS` characters and one path, extension, digit,
 * camelCase, or snake_case token. Entry scoring rejects other notes; the
 * jev scorer builds no summary that fails it.
 */
export function isLocatableSummary(summary: string): boolean {
  return summary.length >= SUMMARY_MIN_CHARS && SUMMARY_ANCHOR.test(summary);
}

/**
 * Scorer calls to report for one `score` result: 0 when the scorer fell back
 * before sending any request (`requested: false`), otherwise 1. Rebuild,
 * entry, and shadow scoring all count with this.
 */
export function requestCount(result: Pick<ScoreResult, "requested">): number {
  return result.requested === false ? 0 : 1;
}

/**
 * The `failedRequests` field every scorer report copies from a result:
 * the count when positive, else nothing, so reports without partial
 * failures keep their shape.
 */
export function failedRequestsField(
  result: Pick<ScoreResult, "failedRequests"> | undefined,
): { failedRequests?: number } {
  const failed = result?.failedRequests ?? 0;
  return failed > 0 ? { failedRequests: failed } : {};
}

/** Cuts a summary note to `SUMMARY_LIMIT` characters. */
export function clampSummary(summary: string): string {
  if (summary.length <= SUMMARY_LIMIT) return summary;
  return summary.slice(0, SUMMARY_LIMIT);
}

/**
 * Builds a request from this turn's compiled view: the session head, the
 * given messages and breakpoints (all cloned), and the latest logged
 * sampling.
 */
export function requestFromCompiled(
  ctx: ViewContext,
  messages: Message[],
  cacheBreakpoints: CompiledView["cacheBreakpoints"],
): Request {
  return {
    model: { provider: ctx.model.provider, model: ctx.model.model },
    head: structuredClone(ctx.head),
    messages: structuredClone(messages),
    cacheBreakpoints: structuredClone(cacheBreakpoints),
    sampling: latestHeader(ctx.path)?.sampling ?? {},
  };
}
