/**
 * The paid half of `sage calibrate-jev`: the answer cache, the spend
 * ledger, and the Jev calls, through the scorer's own `postMany`.
 *
 * Contract: an answer is cached at `<cache>/<sha256 of the body>.json`
 * as soon as it arrives, so a stopped run keeps what it paid for.
 * `spend.json` holds the running total across runs and is rewritten after
 * every answered request. Concurrency, 429/529 backoff, and the
 * rate-limit breaker are `postMany`'s. A failed request is counted and the
 * rest still go out; only an exhausted 429/529 (the breaker) or the
 * cumulative spend passing `maxUsd` stops new requests, and the ones in
 * flight finish and are kept.
 *
 * Intentionally not in the Log: the requests, answers, cache, and spend.
 * They live under the caller's `--out` directory.
 */

import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { finiteOr, isRecord } from "@sage/core";
import { type JevEndpoint, postMany } from "@sage/plugins";
import { listModels } from "@sage/provider";

const TOKENS_PER_MILLION = 1_000_000;

/** One Jev answer as cached: the answers map and the billed usage. */
export type CachedAnswer = {
  answers: Record<string, unknown>;
  inputTokens: number;
  outputTokens: number;
};

/** Token and dollar totals; `usd` is `inputTokens` at the Jev input price. */
export type Spend = {
  requests: number;
  inputTokens: number;
  outputTokens: number;
  usd: number;
};

/** Why a run stopped before sending every request. */
export type SendStop =
  | { kind: "budget" }
  | { kind: "rate-limited"; fallback: string };

/** What one send pass produced. `spent` covers this run only. */
export type SendOutcome = {
  answers: Map<string, CachedAnswer>;
  spent: Spend;
  /** Requests sent that came back without answers. */
  failed: number;
  /** Requests never sent because the breaker opened or the budget ran out. */
  unsent: number;
  stopped?: SendStop;
};

/**
 * What Jev charges per million input tokens, in USD: the
 * `typesafe/jev-latest` catalog row, whose `source` is TypeSafe's System
 * One announcement
 * (https://typesafe.ai/blog/introducing-system-one-models-and-jev).
 * Output tokens are not billed, so estimates and spend count input only.
 * Throws when the catalog has no such row, since no budget gate is then
 * possible.
 */
export function jevInputUsdPerMTok(): number {
  const row = listModels("typesafe").find(
    (entry) => entry.model === "jev-latest",
  );
  if (!row?.pricing) {
    throw new Error(
      "catalog has no typesafe/jev-latest pricing for the calibration budget",
    );
  }
  return row.pricing.inputPerMTok;
}

/** The cache key of a request body: its hex sha256. */
export function requestKey(body: string): string {
  return createHash("sha256").update(body).digest("hex");
}

/** The USD `inputTokens` cost at `usdPerMTok`. */
export function jevUsd(inputTokens: number, usdPerMTok: number): number {
  return (inputTokens * usdPerMTok) / TOKENS_PER_MILLION;
}

/**
 * The cached answer for `key`, or undefined when the file is missing or
 * holds no `answers` object; a malformed file is overwritten on the next
 * paid answer.
 */
export function readCached(
  cacheDir: string,
  key: string,
): CachedAnswer | undefined {
  const parsed = readJsonFile(path.join(cacheDir, `${key}.json`));
  if (!isRecord(parsed) || !isRecord(parsed.answers)) return undefined;
  return {
    answers: parsed.answers,
    inputTokens: finiteOr(parsed.inputTokens, 0),
    outputTokens: finiteOr(parsed.outputTokens, 0),
  };
}

/** The spend recorded in `file` priced at `usdPerMTok`, or zeros. */
export function readSpend(file: string, usdPerMTok: number): Spend {
  const parsed = readJsonFile(file);
  const record = isRecord(parsed) ? parsed : {};
  const inputTokens = finiteOr(record.inputTokens, 0);
  return {
    requests: finiteOr(record.requests, 0),
    inputTokens,
    outputTokens: finiteOr(record.outputTokens, 0),
    usd: jevUsd(inputTokens, usdPerMTok),
  };
}

/**
 * Sends `bodies` (keyed by `requestKey`), caching each answer and writing
 * `prior` plus this run's usage to `spendFile` after each one. A request
 * starts only while `prior.usd` plus this run's spend is at most
 * `maxUsd`. Never throws for a failed request; filesystem errors do throw.
 */
export async function sendRequests(
  bodies: ReadonlyMap<string, string>,
  opts: {
    endpoint: JevEndpoint;
    cacheDir: string;
    spendFile: string;
    prior: Spend;
    maxUsd: number;
    usdPerMTok: number;
  },
): Promise<SendOutcome> {
  mkdirSync(opts.cacheDir, { recursive: true });
  const keys = [...bodies.keys()];
  const outcome: SendOutcome = {
    answers: new Map(),
    spent: emptySpend(),
    failed: 0,
    unsent: 0,
  };
  const overBudget = () => opts.prior.usd + outcome.spent.usd > opts.maxUsd;
  const results = await postMany([...bodies.values()], opts.endpoint, {
    shouldStop: overBudget,
    onPosted(index, posted) {
      const key = keys[index];
      if (!posted.ok || key === undefined) {
        outcome.failed += 1;
        if (!posted.ok && posted.rateLimited) {
          outcome.stopped ??= {
            kind: "rate-limited",
            fallback: posted.fallback,
          };
        }
        return;
      }
      const answer: CachedAnswer = {
        answers: posted.answers,
        inputTokens: posted.inputTokens,
        outputTokens: posted.outputTokens,
      };
      const file = path.join(opts.cacheDir, `${key}.json`);
      writeFileSync(file, `${JSON.stringify(answer)}\n`);
      outcome.answers.set(key, answer);
      addSpend(outcome.spent, answer, opts.usdPerMTok);
      const total = sumSpend(opts.prior, outcome.spent, opts.usdPerMTok);
      writeFileSync(opts.spendFile, `${JSON.stringify(total)}\n`);
      if (overBudget()) outcome.stopped ??= { kind: "budget" };
    },
  });
  const settled = outcome.answers.size + outcome.failed;
  outcome.unsent = results.length - settled;
  return outcome;
}

/** `prior` plus `spent`, priced at `usdPerMTok`. */
export function sumSpend(left: Spend, right: Spend, usdPerMTok: number): Spend {
  const inputTokens = left.inputTokens + right.inputTokens;
  return {
    requests: left.requests + right.requests,
    inputTokens,
    outputTokens: left.outputTokens + right.outputTokens,
    usd: jevUsd(inputTokens, usdPerMTok),
  };
}

function emptySpend(): Spend {
  return { requests: 0, inputTokens: 0, outputTokens: 0, usd: 0 };
}

function addSpend(
  spend: Spend,
  answer: CachedAnswer,
  usdPerMTok: number,
): void {
  spend.requests += 1;
  spend.inputTokens += answer.inputTokens;
  spend.outputTokens += answer.outputTokens;
  spend.usd = jevUsd(spend.inputTokens, usdPerMTok);
}

function readJsonFile(file: string): unknown {
  if (!existsSync(file)) return undefined;
  const text = readFileSync(file, "utf8");
  try {
    return JSON.parse(text);
  } catch {
    // A truncated or hand-edited file reads as absent: the cache refetches
    // that answer and the spend starts over from what the next run pays.
    return undefined;
  }
}
