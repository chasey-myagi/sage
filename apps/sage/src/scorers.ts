/**
 * Builds the scorer a `--scorer` or `--shadow-scorer` name selects.
 *
 * Contract: `rule` is the rule scorer with its default tiers; `model`
 * reuses the session's adapters and model; `jev` calls TypeSafe Jev at
 * `JEV_BASE_URL` (default `https://api.typesafe.ai`) with `JEV_API_KEY`,
 * falling back to `TYPESAFE_API_KEY`, and model `JEV_MODEL` (default
 * `jev-latest`). The caller checks {@link jevApiKey} before building a
 * `jev` scorer.
 *
 * Intentionally not in the Log: the Jev key, base URL, and model name.
 * Scores reach the Log only through the manifest the kernel appends.
 */

import { jevScorer, modelScorer, ruleScorer, type Scorer } from "@sage/plugins";
import type { ModelRef, ProviderAdapter } from "@sage/provider";
import type { ScorerName } from "./options.js";

/** fetch replacement for the Jev scorer. Test seam; production uses global fetch. */
export type JevFetch = NonNullable<
  Parameters<typeof jevScorer>[0]["fetchImpl"]
>;

/** The Jev key: `JEV_API_KEY`, else `TYPESAFE_API_KEY`, else undefined. */
export function jevApiKey(env: NodeJS.ProcessEnv): string | undefined {
  return env.JEV_API_KEY || env.TYPESAFE_API_KEY || undefined;
}

/** The scorer for `name`, or undefined when no name was given. */
export function namedScorer(
  name: ScorerName | undefined,
  adapters: Record<string, ProviderAdapter>,
  model: ModelRef,
  env: NodeJS.ProcessEnv,
  jevFetch?: JevFetch,
): Scorer | undefined {
  if (name === "model") return modelScorer({ adapters, model });
  if (name === "jev") {
    const baseUrl = env.JEV_BASE_URL;
    const jevModel = env.JEV_MODEL;
    return jevScorer({
      apiKey: jevApiKey(env),
      ...(baseUrl ? { baseUrl } : {}),
      ...(jevModel ? { model: jevModel } : {}),
      ...(jevFetch ? { fetchImpl: jevFetch } : {}),
    });
  }
  if (name === "rule") return ruleScorer();
  return undefined;
}
