/**
 * Pieces shared by the three ways the compiler calls a scorer: on rebuild,
 * at entry, and in shadow.
 *
 * Contract: a scorer may only move a tool chunk to none, summary, or full;
 * any other tier in its reply is ignored. A summary note is cut to
 * `SUMMARY_LIMIT` characters. Usage copied onto the manifest has every
 * field coerced to a finite number.
 *
 * Intentionally not in the Log: these helpers and the request they build.
 * The scorer's decisions reach the Log only as manifest rows and reports.
 */

import type { CompiledView, ViewContext } from "@sage/core";
import type { Message, Request, Usage } from "@sage/provider/contract";
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

/** Usage reported when a scorer call made no request. */
export const EMPTY_USAGE: Usage = {
  inputOther: 0,
  inputCacheRead: 0,
  inputCacheCreation: 0,
  output: 0,
};

/** Cuts a summary note to `SUMMARY_LIMIT` characters. */
export function clampSummary(summary: string): string {
  if (summary.length <= SUMMARY_LIMIT) return summary;
  return summary.slice(0, SUMMARY_LIMIT);
}

/** Copies usage with each field coerced to a finite number, else 0. */
export function finiteUsage(usage: Usage): Usage {
  return {
    inputOther: finiteNumber(usage.inputOther),
    inputCacheRead: finiteNumber(usage.inputCacheRead),
    inputCacheCreation: finiteNumber(usage.inputCacheCreation),
    output: finiteNumber(usage.output),
  };
}

function finiteNumber(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
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
