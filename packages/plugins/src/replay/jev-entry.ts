/**
 * Offline jev requests: rebuilds, from a recorded session path, the
 * request the entry-mode jev scorer sends for each tool chunk on the turn
 * it first enters the view, without calling any model.
 *
 * Guarantees: each request comes from `chunkRequest` with the inputs the
 * online entry scorer reads: the chunk as `indexChunks` builds it from the
 * passthrough projection of the turn's prefix, its original tool_result
 * text and the prose of the assistant message that made the call, the
 * turn's latest user query, and the progress ledger of the turn's request
 * rebuilt from the Log. Reads `path` only.
 *
 * Intentionally not in the Log: the requests. They are recomputed from
 * entries the session already wrote.
 */

import { passthroughView, type StoredEntry } from "@sage/core";
import type { Message, Pricing } from "@sage/provider/contract";
import { defaultTokenCount } from "../compiler/budget.js";
import { type Chunk, indexChunks } from "../compiler/chunks.js";
import { EXPAND_TOOL_NAME } from "../compiler/expand.js";
import { latestQuery, projectedSeqs } from "../compiler/log-read.js";
import { rebuildFromLog } from "../compiler/rebuild.js";
import { DEFAULT_MIN_SCORE_TOKENS } from "../compiler/score-entry.js";
import { readSources } from "../scorer/jev/sources.js";
import { type ChunkRequest, chunkRequest } from "../scorer/jev/state.js";
import { buildViewContext, sliceTurns, type TurnSlice } from "./offline.js";

/** One chunk's rebuilt request and the turn facts calibration prices it with. */
export type EntryJevRequest = {
  chunkId: string;
  tool: string;
  /** The compiler's token estimate of the whole chunk. */
  tokens: number;
  /** Turn on which the chunk first entered the view. */
  turn: number;
  /**
   * Path index where that turn's view commit starts; a call at a larger
   * index came after the entry decision.
   */
  decidedAt: number;
  /** Turns with a reply after the entry turn. */
  remainingTurns: number;
  /** The entry turn's `request_header.pricing`. */
  pricing: Pricing | null;
  request: ChunkRequest;
};

/**
 * Requests for every new tool chunk, plus what was left out: `isError`
 * chunks (the scorer decides them full without asking) and chunks with no
 * result or no request that fits Jev's limits.
 */
export type EntryJevScan = {
  requests: EntryJevRequest[];
  skipped: { isError: number; unsendable: number };
};

/**
 * Walks the turns of `path` (one per request_header with a reply) and
 * rebuilds the jev request for each tool chunk the first time it appears,
 * when it is not an `expand` result and has at least `minScoreTokens`
 * (default `DEFAULT_MIN_SCORE_TOKENS`). Unlike the online candidate filter
 * it does not require the compiled row to still be full, so a chunk the
 * budget had already cut is asked about as well. A passthrough session
 * has no claims, so its ledger is always "" and Jev sees a different
 * input distribution than in a compiler session; a turn whose request
 * cannot be rebuilt also gets "".
 */
export async function entryJevRequests(
  path: readonly StoredEntry[],
  opts: { minScoreTokens?: number } = {},
): Promise<EntryJevScan> {
  const minTokens = opts.minScoreTokens ?? DEFAULT_MIN_SCORE_TOKENS;
  const slices = sliceTurns(path);
  const seen = new Set<string>();
  const scan: EntryJevScan = {
    requests: [],
    skipped: { isError: 0, unsendable: 0 },
  };
  for (const [position, slice] of slices.entries()) {
    const prefix = path.slice(0, slice.prefixEnd);
    const { messages, chunks } = await projectTurn(prefix, slice);
    const fresh = chunks.filter((chunk) => !seen.has(chunk.id));
    for (const chunk of chunks) seen.add(chunk.id);
    const candidates = fresh.filter((chunk) => isCandidate(chunk, minTokens));
    if (candidates.length === 0) continue;
    const history = readSources(messages);
    const context = {
      task: latestQuery(prefix),
      ledger: ledgerAt(path, slice.turn),
    };
    const facts = {
      turn: slice.turn,
      decidedAt: slice.prefixEnd,
      remainingTurns: slices.length - 1 - position,
      pricing: slice.header.pricing,
    };
    for (const chunk of candidates) {
      if (chunk.isError) {
        scan.skipped.isError += 1;
        continue;
      }
      const text = history.resultText(chunk);
      const prose = history.assistantProse(chunk);
      const request =
        text === undefined
          ? undefined
          : chunkRequest(chunk, text, { ...context, prose });
      if (!request) {
        scan.skipped.unsendable += 1;
        continue;
      }
      const tool = chunk.toolName ?? "";
      scan.requests.push({
        chunkId: chunk.id,
        tool,
        tokens: chunk.tokens,
        ...facts,
        request,
      });
    }
  }
  return scan;
}

/** A turn prefix's passthrough messages and their chunks, indexed as the compiler does. */
async function projectTurn(
  prefix: readonly StoredEntry[],
  slice: TurnSlice,
): Promise<{ messages: Message[]; chunks: Chunk[] }> {
  const ctx = buildViewContext(slice.turn, slice.header, prefix, {
    tools: [],
    system: "",
  });
  const base = await passthroughView.view(ctx);
  const seqs = projectedSeqs(prefix, base.manifest.chunks);
  return {
    messages: base.messages,
    chunks: indexChunks(base.messages, seqs, [], defaultTokenCount),
  };
}

function isCandidate(chunk: Chunk, minTokens: number): boolean {
  return (
    chunk.kind === "tool" &&
    chunk.toolName !== EXPAND_TOOL_NAME &&
    chunk.tokens >= minTokens
  );
}

/** The progress ledger in the request rebuilt for `turn`, or "". */
function ledgerAt(path: readonly StoredEntry[], turn: number): string {
  let messages: Message[];
  try {
    messages = rebuildFromLog(path, turn).messages;
  } catch {
    // A turn whose manifest or claim content is missing cannot be rebuilt;
    // without the rebuilt request there is no ledger to read, the same ""
    // the online scorer sends for a view without the claim.
    return "";
  }
  return readSources(messages).ledger;
}
