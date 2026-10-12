/**
 * Jev scorer: one TypeSafe System One request per grep or bash-runner
 * candidate chunk, the chunks the rules would drop. Jev answers Noul
 * questions about an excerpt of the chunk's original output; code turns
 * the answers into tiers. Other chunks get no decision.
 *
 * Guarantees: the scorer never throws. A failed request costs only its
 * own chunk, which gets no decision; only when no request is answered
 * does the call resolve `{ decisions: [], fallback }`, so the compiler
 * keeps the deterministic tiers. The original output comes only from
 * `prevRequest`; no other history is sent. Request fields, tiers, and
 * retries: `jevScorer`.
 *
 * Intentionally not in the Log: these requests and their answers. The
 * compiler copies decisions (probabilities in the reason), usage, and
 * fallback onto the manifest.
 */

import { emptyUsage, type Sleeper, type Usage } from "@sage/provider/contract";
import { type JevEndpoint, type JevFetch, postMany } from "./jev/http.js";
import { questionSetFor, readJudgment } from "./jev/questions.js";
import { readSources } from "./jev/sources.js";
import { type ChunkRequest, chunkRequest, requestBody } from "./jev/state.js";
import {
  DEFAULT_JEV_THRESHOLDS,
  decideTier,
  errorDecision,
  type JevThresholds,
} from "./jev/tiers.js";
import type {
  ScoreDecision,
  ScoreInput,
  ScoreResult,
  Scorer,
} from "./types.js";

/** Jev model the scorer names when `model` is empty. */
export const JEV_MODEL_ID = "jev-latest";
const DEFAULT_TIMEOUT_MS = 10_000;

/**
 * Creates the jev scorer. Its `name` is `jev`.
 *
 * `score` POSTs one `{ state, model, questions }` per candidate to
 * `<baseUrl>/v1/systemone` (default `https://api.typesafe.ai`) with
 * `Authorization: Bearer <apiKey>`; at most 4 are in flight. `model` is
 * `opts.model` when non-empty, else `jev-latest`. `state` is `{ task,
 * ledger, recent_assistant, chunk }`: the query, the newest
 * `progress-ledger` claim body, and the text of the assistant message that
 * made the call, each clipped; `chunk` is `{ id, tool, input, tokens,
 * head, tail, omitted_lines, candidates }` with the first 40 and last 20
 * output lines (none when the candidates hold the whole output) and up to
 * 24 candidate lines picked by tool name.
 *
 * Only grep chunks and bash chunks `classifyBashCommand` calls a runner or
 * mixed are triaged (`questionSetFor`). Their questions are the Nouls
 * `reports_failure`, `locates_next_step`, and `keep_<i>` per candidate.
 * Every other chunk, read included, is not sent and gets no decision, so
 * the compiler's rules decide it; the read probe (`shows_target_code`) is
 * asked only by offline calibration.
 *
 * A triaged chunk whose result is `isError` is not sent; it is full with
 * reason `scorer:jev:full:is_error`. Other tiers come from `opts.thresholds`
 * over `DEFAULT_JEV_THRESHOLDS`: full when a failure report or a new
 * location reaches `full`; none when the failure, the location, and every
 * keep answer are below `low`; else summary quoting up to three
 * candidates at or above `keep`, or full when no locatable note results.
 * The reason is `scorer:jev:<tier>:failure=…,locate=…[,keep=…]`.
 *
 * Sources: the original output is the chunk's tool_result in
 * `prevRequest.messages`. Entry and shadow scoring pass this turn's view,
 * so every candidate has it; rebuild scoring passes the previous request,
 * where a chunk stubbed last epoch has only its stub. A chunk with no
 * original output, or whose request cannot fit Jev's limits, is not sent
 * and gets no decision, so the compiler leaves it as it was. When no
 * chunk is sent, score resolves `{ decisions: [], requested: false }`.
 *
 * Usage sums `input_tokens` into `inputOther` and `output_tokens` into
 * `output`. Never throws. A missing `apiKey` resolves `no-api-key` and a
 * missing `prevRequest` `no-previous-request`, both with `requested:
 * false`. A failed request (status, timeout `timeoutMs` default 10000,
 * abort, or a body without answers) gives its chunk no decision and adds
 * 1 to `failedRequests`; the other requests still go out and their
 * decisions stand, except that once one request exhausts its 429/529
 * retries, or `signal` aborts, requests not yet started are not sent and
 * count as failed with that reason. Only when every request fails does the call resolve
 * the first failure's fallback, with no decisions and no usage. 429 and
 * 529 retry at most 3 times, sleeping for the response's `retry-after`
 * when it has one, else `500 × 2^n` ms; a caller abort ends the wait with
 * `aborted`. A chunk whose answers lack a fixed judgment gets no
 * decision.
 */
export function jevScorer(opts: {
  apiKey?: string;
  baseUrl?: string;
  model?: string;
  /** Test seam. Production uses the global `fetch`. */
  fetchImpl?: JevFetch;
  /** Test seam. Production uses `sleep` from the provider contract. */
  sleepImpl?: Sleeper;
  thresholds?: Partial<JevThresholds>;
  timeoutMs?: number;
}): Scorer {
  const apiKey = opts.apiKey ?? "";
  const model = opts.model || JEV_MODEL_ID;
  const thresholds = { ...DEFAULT_JEV_THRESHOLDS, ...opts.thresholds };
  const endpoint = (signal: AbortSignal | undefined): JevEndpoint => ({
    apiKey,
    ...(opts.baseUrl !== undefined ? { baseUrl: opts.baseUrl } : {}),
    timeoutMs: opts.timeoutMs ?? DEFAULT_TIMEOUT_MS,
    ...(signal ? { signal } : {}),
    ...(opts.fetchImpl ? { fetchImpl: opts.fetchImpl } : {}),
    ...(opts.sleepImpl ? { sleepImpl: opts.sleepImpl } : {}),
  });

  return {
    name: "jev",
    async score(input) {
      if (!apiKey) {
        return { decisions: [], fallback: "no-api-key", requested: false };
      }
      if (input.candidates.length === 0) return { decisions: [] };
      try {
        const built = buildRequests(input);
        if (built === undefined) {
          return {
            decisions: [],
            fallback: "no-previous-request",
            requested: false,
          };
        }
        if (built.requests.length === 0) {
          return { decisions: built.decided, requested: false };
        }
        const posted = await postRequests(built.requests, {
          endpoint: endpoint(input.signal),
          model,
          thresholds,
        });
        if (posted.fallback !== undefined) return posted;
        return {
          ...posted,
          decisions: [...built.decided, ...posted.decisions],
        };
      } catch {
        // Scorers never throw: a bug in building requests or reading
        // answers becomes a fallback, and the compiler keeps its tiers.
        return { decisions: [], fallback: "error" };
      }
    },
  };
}

/**
 * One request per triaged candidate with original output, and the
 * decisions made without asking (triaged `isError` chunks); undefined
 * without `prevRequest`.
 */
function buildRequests(
  input: ScoreInput,
): { requests: ChunkRequest[]; decided: ScoreDecision[] } | undefined {
  if (!input.prevRequest) return undefined;
  const sources = readSources(input.prevRequest.messages);
  const requests: ChunkRequest[] = [];
  const decided: ScoreDecision[] = [];
  for (const chunk of input.candidates) {
    const set = questionSetFor(chunk);
    // A read probe decides nothing, so online it would only cost tokens.
    if (set !== "triage") continue;
    if (chunk.isError) {
      decided.push(errorDecision(chunk.id));
      continue;
    }
    const text = sources.resultText(chunk);
    if (text === undefined) continue;
    const request = chunkRequest(
      chunk,
      text,
      {
        task: input.query,
        ledger: sources.ledger,
        prose: sources.assistantProse(chunk),
      },
      set,
    );
    if (request) requests.push(request);
  }
  return { requests, decided };
}

/**
 * Sends every request through `postMany`. A failed request costs only its
 * own chunk: the others keep their decisions, and the failures, including
 * requests the rate-limit breaker or an abort kept from being sent, are
 * counted in `failedRequests`. The result falls back, with the first
 * failure's reason in request order, only when no request is answered.
 */
async function postRequests(
  requests: readonly ChunkRequest[],
  opts: { endpoint: JevEndpoint; model: string; thresholds: JevThresholds },
): Promise<ScoreResult> {
  const results = await postMany(
    requests.map((request) => requestBody(request, opts.model)),
    opts.endpoint,
  );
  const usage: Usage = emptyUsage();
  let failed = 0;
  let firstFallback: string | undefined;
  for (const posted of results) {
    if (!posted?.ok) {
      failed += 1;
      firstFallback ??= posted?.fallback;
      continue;
    }
    usage.inputOther += posted.inputTokens;
    usage.output += posted.outputTokens;
  }
  if (failed === requests.length) {
    return { decisions: [], fallback: firstFallback ?? "error" };
  }
  return {
    decisions: requests.flatMap((request, index) => {
      const posted = results[index];
      const answers = posted?.ok ? posted.answers : undefined;
      return decisionFor(request, answers, opts.thresholds);
    }),
    usage,
    ...(failed > 0 ? { failedRequests: failed } : {}),
  };
}

function decisionFor(
  request: ChunkRequest,
  answers: Record<string, unknown> | undefined,
  thresholds: JevThresholds,
): ScoreDecision[] {
  if (!answers) return [];
  const judgment = readJudgment(
    answers,
    request.set,
    request.candidates.length,
  );
  if (judgment?.set !== "triage") return [];
  return [
    decideTier(
      request.state.chunk.id,
      judgment,
      request.candidates,
      thresholds,
    ),
  ];
}
