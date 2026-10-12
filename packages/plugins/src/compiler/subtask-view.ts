/**
 * Subtask recompile: a child session's view over its parent's frozen
 * request.
 *
 * Contract: the child's messages start with the parent's latest request
 * rebuilt from the Log, byte for byte, epoch head included, so the child
 * hits the parent's prefix cache. The child never opens an epoch: it
 * reports the parent's current epoch and a fixed no-rebuild decision.
 * Throws when the parent path has no manifest, or when the passthrough
 * projection is not synchronous.
 *
 * Intentionally not in the Log: the parent messages rebuilt here. The
 * child's manifest records the parent rows and the query claim text.
 */

import type { CompiledView, ViewContext } from "@sage/core";
import { isRecord, isTier, latestViewEpoch, passthroughView } from "@sage/core";
import type { Message } from "@sage/provider/contract";
import { escapeClaimText } from "../claims/escape.js";
import { applyDecisions } from "./apply.js";
import type { TokenCounter } from "./budget.js";
import { measuredCacheRead } from "./calibrate.js";
import type { Chunk } from "./chunks.js";
import { indexChunks, OWN_CHUNK_PREFIX } from "./chunks.js";
import type { EpochDecision } from "./decide.js";
import type { ChunkDecision } from "./eliminate.js";
import { eliminate } from "./eliminate.js";
import { contentHash, renderClaim } from "./format.js";
import { frozenDecisions } from "./freeze.js";
import {
  assertSeqsMatch,
  countExpands,
  latestManifest,
  manifestTurn,
  type PathEntry,
  projectionSeqs,
} from "./log-read.js";
import { rebuildFromLog } from "./rebuild.js";
import {
  type ChunkRow,
  firstById,
  keptRow,
  type ManifestClaim,
  stubRow,
} from "./rows.js";

/** Claim id of the child's question, rendered as a tail claim. */
const SUBTASK_QUERY_CLAIM_ID = "subtask-query";
/** Priority recorded for the pinned subtask-query claim. */
const SUBTASK_QUERY_PRIORITY = 100;

/** Settings the subtask compile inherits from `contextCompiler`. */
export type SubtaskCompileOptions = {
  budget: number;
  query: string;
  count: TokenCounter;
  keepRecent: number;
};

/**
 * Compiles a child view. Parent chunk tiers come from the parent's latest
 * manifest, overlaid with `frozenDecisions` of the parent's current epoch.
 * The child's own messages become `own:` chunks and go through `eliminate`
 * with what the parent rows and the query leave of the budget. The
 * `subtask-query` tail claim carries `query` and is appended after the
 * parent prefix. `decideEpoch` is not called; the decision reason is
 * `subtask:no-rebuild`. `budget.frozenTokens` and `budget.staleOmittable`
 * are 0 because this compile does not compare a reopen. The shadow scorer
 * does not run.
 */
export function compileSubtask(
  ctx: ViewContext,
  opts: SubtaskCompileOptions,
): CompiledView {
  const parent = parentPrefix(ctx.parentPath ?? []);
  const own = ownChunks(ctx, opts.count);
  const rendered = renderClaim(
    SUBTASK_QUERY_CLAIM_ID,
    "full",
    escapeClaimText(opts.query),
  );
  const claimTokens = opts.count(rendered);
  const parentTokens = keptTokens(parent.rows);
  const eliminated = eliminate(own.chunks, {
    keepRecent: opts.keepRecent,
    remainingBudget: Math.max(0, opts.budget - parentTokens - claimTokens),
  });
  const ownMessages = applyDecisions(
    own.messages,
    own.chunks,
    eliminated.decisions,
  );
  const parentCount = parent.messages.length;
  const messages = [...parent.messages, ...ownMessages];
  appendSubtaskQuery(messages, parentCount, rendered);
  return subtaskView(ctx, opts, {
    parent,
    own,
    messages,
    parentCount,
    ownRows: ownRowsFrom(own.chunks, eliminated.decisions, opts.count),
    rendered,
    claimTokens,
    parentTokens,
    used: parentTokens + eliminated.used + claimTokens,
  });
}

/**
 * Builds the child view and manifest. The prefix breakpoint sits on the
 * last parent message.
 */
function subtaskView(
  ctx: ViewContext,
  opts: SubtaskCompileOptions,
  input: {
    parent: ParentPrefix;
    own: OwnChunks;
    /** All messages; the first `parentCount` are the parent prefix. */
    messages: Message[];
    parentCount: number;
    ownRows: ChunkRow[];
    rendered: string;
    claimTokens: number;
    parentTokens: number;
    used: number;
  },
): CompiledView {
  const { parent, ownRows, used } = input;
  const omitted =
    parent.rows.filter((row) => row.tier === "none").length +
    ownRows.filter((row) => row.tier === "none").length;
  return {
    epoch: parent.epoch,
    head: ctx.head,
    messages: input.messages,
    cacheBreakpoints: [
      "head",
      { afterMessage: Math.max(0, input.parentCount - 1) },
      { afterMessage: Math.max(0, input.messages.length - 1) },
    ],
    manifest: {
      epoch: parent.epoch,
      turn: ctx.turn,
      budget: {
        total: opts.budget,
        used,
        overrun: Math.max(0, used - opts.budget),
        omitted,
        expands: countExpands(ctx.path),
        frozenTokens: 0,
        staleOmittable: 0,
      },
      claims: [queryClaimRecord(input.rendered, input.claimTokens)],
      chunks: [...parent.rows, ...ownRows],
      anomalies: input.own.anomalies,
      epochDecision: noRebuildDecision(input.parentTokens, used),
      cache: {
        expectedReadTokens: input.parentTokens,
        measuredReadTokens: measuredCacheRead(ctx.parentPath ?? []),
      },
    },
    details: { claimContent: { [SUBTASK_QUERY_CLAIM_ID]: input.rendered } },
  };
}

function queryClaimRecord(rendered: string, tokens: number): ManifestClaim {
  return {
    id: SUBTASK_QUERY_CLAIM_ID,
    tier: "full",
    reason: "pinned",
    tokens,
    placement: "tail",
    priority: SUBTASK_QUERY_PRIORITY,
    contentHash: contentHash(rendered),
  };
}

type ParentPrefix = { epoch: number; messages: Message[]; rows: ChunkRow[] };

function parentPrefix(parentPath: readonly PathEntry[]): ParentPrefix {
  const epoch = latestViewEpoch(parentPath);
  const manifest = latestManifest(parentPath);
  const turn = manifestTurn(manifest);
  if (turn === undefined) {
    throw new Error("subtask parent path has no manifest");
  }
  const rebuilt = rebuildFromLog(parentPath, turn);
  return {
    epoch,
    messages: structuredClone(rebuilt.messages),
    rows: overlayFrozen(
      chunkRows(manifest),
      frozenDecisions(parentPath, epoch),
    ),
  };
}

type OwnChunks = {
  chunks: Chunk[];
  messages: Message[];
  anomalies: CompiledView["manifest"]["anomalies"];
};

function ownChunks(ctx: ViewContext, count: TokenCounter): OwnChunks {
  const projected = passthroughView.view(ctx);
  if (projected instanceof Promise) {
    throw new Error("passthrough projection must be synchronous");
  }
  const seqs = projectionSeqs(ctx.path);
  assertSeqsMatch(seqs.length, projected.messages.length);
  const chunks = indexChunks(projected.messages, seqs, ctx.tools, count).map(
    (chunk) => ({ ...chunk, id: `${OWN_CHUNK_PREFIX}${chunk.id}` }),
  );
  return {
    chunks,
    messages: projected.messages,
    anomalies: projected.manifest.anomalies,
  };
}

function noRebuildDecision(parentTokens: number, used: number): EpochDecision {
  return {
    rebuild: false,
    oldTokens: parentTokens,
    newTokens: used,
    remainingTurns: 1,
    readRate: 0,
    writeRate: 0,
    savingsPerTurn: 0,
    extraCost: 0,
    paybackTurns: null,
    reason: "subtask:no-rebuild",
  };
}

function chunkRows(entry: PathEntry | undefined): ChunkRow[] {
  if (
    !entry ||
    !isRecord(entry.payload) ||
    !Array.isArray(entry.payload.chunks)
  ) {
    return [];
  }
  const rows: ChunkRow[] = [];
  for (const value of entry.payload.chunks) {
    if (!isRecord(value) || typeof value.id !== "string") continue;
    if (!isTier(value.tier)) continue;
    const row: ChunkRow = {
      id: value.id,
      tier: value.tier,
      reason: typeof value.reason === "string" ? value.reason : "kept",
      tokens: typeof value.tokens === "number" ? value.tokens : 0,
    };
    if (typeof value.originalTokens === "number") {
      row.originalTokens = value.originalTokens;
    }
    rows.push(row);
  }
  return rows;
}

function overlayFrozen(
  rows: ChunkRow[],
  frozen: Map<string, ChunkDecision> | undefined,
): ChunkRow[] {
  if (!frozen) return rows;
  return rows.map((row) => {
    const kept = frozen.get(row.id);
    if (!kept) return row;
    return { ...row, tier: kept.tier, reason: kept.reason };
  });
}

function keptTokens(rows: readonly ChunkRow[]): number {
  return rows.reduce(
    (sum, row) => (row.tier === "none" ? sum : sum + row.tokens),
    0,
  );
}

function ownRowsFrom(
  chunks: readonly Chunk[],
  decisions: readonly ChunkDecision[],
  count: TokenCounter,
): ChunkRow[] {
  const chunkById = firstById(chunks);
  return decisions.map((decision) => {
    const chunk = chunkById.get(decision.id);
    if (decision.tier === "none" && chunk?.kind === "tool") {
      return stubRow(chunk, decision.reason, count);
    }
    return keptRow(
      decision.id,
      decision.tier,
      decision.reason,
      chunk?.tokens ?? 0,
    );
  });
}

function appendSubtaskQuery(
  messages: Message[],
  parentCount: number,
  rendered: string,
): void {
  const block = { type: "text" as const, text: rendered };
  const last = messages.at(-1);
  if (messages.length > parentCount && last?.role === "user") {
    last.content.push(block);
    return;
  }
  messages.push({ role: "user", content: [block] });
}
