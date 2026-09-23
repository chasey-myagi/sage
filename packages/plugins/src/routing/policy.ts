/**
 * First-party route policies. Pure functions over the compiler's input.
 *
 * Contract: `composePolicies` feeds the previous model forward as `current`.
 * Undefined keeps the previous result and the previous current.
 * `sensitivePolicy` drops providers when a chunk whose tier is not `none`
 * has an access key matching a rule. `costTierPolicy` picks the ladder
 * entry with the lowest catalog input price that is still in `candidates`.
 * Ladder entries the catalog cannot price are skipped. Models off the
 * ladder are not considered.
 *
 * Intentionally not in the Log: these functions. Reasons reach the manifest
 * only when the compiler appends them.
 */

import { type CatalogEntry, listModels } from "@sage/provider";
import type { RouteInput, RoutePolicy, RouteResult } from "./types.js";

/**
 * Runs `policies` in order. Each sees the previous chosen model as
 * `current`; one returning undefined keeps the previous result and current.
 * Returns the last defined result, or undefined when none chose.
 */
export function composePolicies(...policies: RoutePolicy[]): RoutePolicy {
  return (input) => {
    let current = input.current;
    let chosen: RouteResult;
    for (const policy of policies) {
      const next = policy({ ...input, current });
      if (!next) continue;
      current = next.model;
      chosen = next;
    }
    return chosen;
  };
}

/**
 * Creates a candidate filter. Any view chunk whose tier is not `none` and
 * has an access key matching a rule removes that rule's providers from the
 * candidates and adds the reason `sensitive:<key>` once per key. Each
 * pattern's `lastIndex` is reset before testing.
 */
export function sensitivePolicy(opts: {
  rules: { pattern: RegExp; denyProviders: string[] }[];
}): (input: RouteInput) => { candidates: CatalogEntry[]; reasons: string[] } {
  return (input) => {
    const denied = new Set<string>();
    const reasons: string[] = [];
    const seen = new Set<string>();
    for (const chunk of input.viewChunks) {
      if (chunk.tier === "none") continue;
      for (const access of chunk.accesses ?? []) {
        for (const rule of opts.rules) {
          rule.pattern.lastIndex = 0;
          if (!rule.pattern.test(access.key)) continue;
          for (const provider of rule.denyProviders) denied.add(provider);
          const reason = `sensitive:${access.key}`;
          if (seen.has(reason)) continue;
          seen.add(reason);
          reasons.push(reason);
        }
      }
    }
    return {
      candidates: input.candidates.filter(
        (entry) => !denied.has(entry.provider),
      ),
      reasons,
    };
  };
}

/**
 * Runs `policy` on the filtered candidates. Returns undefined when the
 * policy abstains or picks a model the filter removed; otherwise prefixes
 * the filter's reasons, `;`-joined, to the policy reason.
 */
export function withSensitiveFilter(
  policy: RoutePolicy,
  filter: (input: RouteInput) => {
    candidates: CatalogEntry[];
    reasons: string[];
  },
): RoutePolicy {
  return (input) => {
    const filtered = filter(input);
    const result = policy({ ...input, candidates: filtered.candidates });
    if (!result) return undefined;
    const allowed = filtered.candidates.some(
      (entry) =>
        entry.provider === result.model.provider &&
        entry.model === result.model.model,
    );
    if (!allowed) return undefined;
    if (filtered.reasons.length === 0) return result;
    return {
      model: result.model,
      reason: `${filtered.reasons.join(";")};${result.reason}`,
    };
  };
}

/**
 * Picks the ladder entry with the lowest catalog input price that is still
 * a candidate; reason `cheapest-allowed:<provider>/<model>`. Unpriced
 * entries and models off the ladder are skipped; returns undefined when no
 * entry qualifies.
 */
export function costTierPolicy(opts: {
  ladder: { provider: string; model: string }[];
  pick: "cheapest-allowed";
}): RoutePolicy {
  return (input) => {
    if (opts.pick !== "cheapest-allowed") return undefined;
    let best: { provider: string; model: string; price: number } | undefined;
    for (const ref of opts.ladder) {
      const offered = input.candidates.some(
        (entry) => entry.provider === ref.provider && entry.model === ref.model,
      );
      if (!offered) continue;
      const priced = listModels(ref.provider).find(
        (entry) => entry.model === ref.model,
      );
      const price = priced?.pricing.inputPerMTok;
      if (price === undefined || !Number.isFinite(price)) continue;
      if (best && price >= best.price) continue;
      best = { provider: ref.provider, model: ref.model, price };
    }
    if (!best) return undefined;
    return {
      model: { provider: best.provider, model: best.model },
      reason: `cheapest-allowed:${best.provider}/${best.model}`,
    };
  };
}
