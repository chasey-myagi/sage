/**
 * Assembles one candidate view: claims placed and allocated, chunk tiers
 * decided, messages rendered, manifest rows built.
 *
 * Contract: `assembleView` is pure over its input. It never mutates the
 * projected history or the Log path, and two calls with the same input
 * produce byte-identical output. The compiler calls it once for the frozen
 * view and, inside an epoch, once more for a from-scratch recompile, then
 * picks one.
 *
 * Intentionally not in the Log: claim providers, the live `Claim` objects,
 * and the rejected candidate view. The chosen assembly's rows, claim
 * records, and new claim text reach the Log through the manifest.
 */

import { errorText, type ViewContext } from "@sage/core";
import type { Message } from "@sage/provider/contract";
import { applyDecisions, renderSummary } from "./apply.js";
import { allocate, type TokenCounter } from "./budget.js";
import type { FrozenMap } from "./chunk-decisions.js";
import { decideChunks } from "./chunk-decisions.js";
import type { Chunk } from "./chunks.js";
import type { Claim, ClaimProvider, Tier } from "./claim.js";
import type { ChunkDecision } from "./eliminate.js";
import { contentHash, parseClaimMessage, renderClaim } from "./format.js";
import {
  epochHeadFor,
  expandStamps,
  isLoggedMessage,
  latestManifest,
  loggedSummaries,
  previousHash,
  type SessionPath,
} from "./log-read.js";
import { withStamps } from "./reasons.js";
import {
  type ChunkRow,
  keptRow,
  type ManifestClaim,
  stubRow,
  summaryRow,
} from "./rows.js";

/** Epoch-start reason when a turn renders a new epoch-head message. */
const FIRST_TURN_EPOCH_START = "compiler:first-turn";

/** Claims from every provider, plus one error row per provider that threw. */
export type CollectedClaims = { claims: Claim[]; errors: ManifestClaim[] };

/** One candidate view and the manifest facts that describe it. */
export type Assembly = {
  messages: Message[];
  /** Claim rows sorted by priority, followed by provider error rows. */
  records: ManifestClaim[];
  chunkRecords: ChunkRow[];
  /** Estimated tokens of claims plus chunks, before calibration. */
  used: number;
  /** Rendered claim text whose hash differs from the previous manifest. */
  claimContent: Record<string, string>;
  /** Set when this assembly rendered a new epoch-head message. */
  epochStart?: { reason: string };
  /** True when `messages[0]` is the epoch-head message. */
  hasEpochHead: boolean;
};

/** Everything `assembleView` reads; shared by both candidate views. */
export type AssembleInput = {
  path: SessionPath;
  /** Passthrough projection of the history. Not mutated. */
  history: readonly Message[];
  epoch: number;
  collected: CollectedClaims;
  chunks: Chunk[];
  /** First-wins index of `chunks`. */
  chunkById: ReadonlyMap<string, Chunk>;
  budget: number;
  keepRecent: number;
  count: TokenCounter;
};

/**
 * How to assemble: `freezeHead` reuses the epoch's stored head message;
 * `frozen` holds the epoch's frozen chunk decisions, if any.
 */
export type AssembleMode = {
  freezeHead: boolean;
  frozen: FrozenMap | undefined;
};

/**
 * Calls every claim provider concurrently. A provider that throws becomes
 * one manifest row `provider:<index>` at tier none, reason
 * `provider_error:<message>`, and does not stop the compile.
 */
export async function collectClaims(
  providers: readonly ClaimProvider[],
  ctx: ViewContext,
): Promise<CollectedClaims> {
  const claims: Claim[] = [];
  const errors: ManifestClaim[] = [];
  const settled = await Promise.all(
    providers.map(async (provider, index) => {
      try {
        return { index, claims: await provider(ctx) };
      } catch (error) {
        return { index, message: errorText(error) };
      }
    }),
  );
  for (const item of settled) {
    if ("message" in item && item.message !== undefined) {
      errors.push({
        id: `provider:${item.index}`,
        tier: "none",
        reason: `provider_error:${item.message}`,
        tokens: 0,
      });
      continue;
    }
    if ("claims" in item) claims.push(...item.claims);
  }
  return { claims, errors };
}

/**
 * Builds one candidate view.
 *
 * The epoch-head message is rendered once per epoch. With `freezeHead`,
 * later turns reuse `view_epoch.details.epochHead` unless that message is
 * itself a logged history message, and ignore the providers' current
 * epoch-head text. Otherwise a non-empty head is rendered and
 * `epochStart.reason` is `compiler:first-turn`. Claims get the budget
 * first; chunks share what is left. Tail claims are recomputed every turn
 * and appended to the last message when it is a user message, else as a
 * new user message. Claims at tier none are omitted from the view.
 */
export function assembleView(
  input: AssembleInput,
  mode: AssembleMode,
): Assembly {
  const epochHead = mode.freezeHead ? reusableEpochHead(input) : undefined;
  const claims = placeClaims(input, epochHead);
  const decided = decideChunks(
    input.chunks,
    mode.frozen,
    input.keepRecent,
    Math.max(0, input.budget - claims.used),
  );
  if (mode.frozen) {
    attachLoggedSummaries(
      decided.decisions,
      loggedSummaries(input.path, input.epoch),
    );
  }
  const messages = applyDecisions(
    input.history,
    input.chunks,
    decided.decisions,
  );
  const chunkRecords = chunkRows(input, decided.decisions);
  const chunkUsed = decided.used + summaryTokenDelta(input, decided.decisions);
  let epochStart: { reason: string } | undefined;
  if (epochHead) {
    messages.unshift(epochHead);
  } else {
    const head = renderEpochHead(claims.records, claims.claimById);
    if (head) {
      messages.unshift(head);
      epochStart = { reason: FIRST_TURN_EPOCH_START };
    }
  }
  appendTail(messages, tailBlocks(claims.records, claims.claimById));
  return {
    messages,
    records: [...claims.records, ...input.collected.errors],
    chunkRecords,
    used: claims.used + chunkUsed,
    claimContent: claims.claimContent,
    ...(epochStart ? { epochStart } : {}),
    hasEpochHead: epochHead !== undefined || epochStart !== undefined,
  };
}

function reusableEpochHead(input: AssembleInput): Message | undefined {
  const stored = epochHeadFor(input.path, input.epoch);
  if (!stored || isLoggedMessage(input.path, stored)) return undefined;
  return stored;
}

type ClaimPlacement = {
  records: ManifestClaim[];
  claimById: ReadonlyMap<string, Claim>;
  claimContent: Record<string, string>;
  used: number;
};

/**
 * With a reused epoch head, epoch-head claims keep the tier and text frozen
 * in that message (reason `frozen:epoch-head`) and are not re-allocated;
 * their tokens are charged first. Every other claim goes through
 * `allocate` with the rest of the budget.
 */
function placeClaims(
  input: AssembleInput,
  epochHead: Message | undefined,
): ClaimPlacement {
  const headParts = epochHead ? parseClaimMessage(epochHead) : undefined;
  const allocatable: Claim[] = [];
  const headClaims: Claim[] = [];
  for (const claim of input.collected.claims) {
    if (epochHead && claim.placement === "epoch-head") {
      headClaims.push(claim);
    } else allocatable.push(claim);
  }
  const headUsed = headParts ? tokensOf(headParts, input.count) : 0;
  const allocation = allocate(
    allocatable,
    Math.max(0, input.budget - headUsed),
    input.count,
  );
  const claimById = new Map(allocatable.map((claim) => [claim.id, claim]));
  const records: ManifestClaim[] = [];
  for (const claim of headClaims) {
    records.push(frozenRecord(claim, headParts, input.count));
  }
  for (const record of allocation.records) {
    const claim = claimById.get(record.id);
    if (!claim) continue;
    records.push({
      ...record,
      placement: claim.placement,
      priority: claim.priority,
    });
  }
  records.sort(byPriority);
  const claimContent = recordClaimContent(
    records,
    claimById,
    latestManifest(input.path),
  );
  const allocatedUsed = allocation.records.reduce(
    (sum, record) => sum + (record.tokens ?? 0),
    0,
  );
  return {
    records,
    claimById,
    claimContent,
    used: headUsed + allocatedUsed,
  };
}

/**
 * Sets `contentHash` on every rendered allocatable claim and returns the
 * text whose hash differs from `previous`. A `tail-if-changed` claim whose
 * hash matches gets reason `unchanged` and is still shown; one whose hash
 * differs from a recorded hash gets reason `changed`. Epoch-head claims
 * get a hash but no text here: their text is stored on `view_epoch`.
 * Mutates `records`.
 */
function recordClaimContent(
  records: ManifestClaim[],
  claimById: ReadonlyMap<string, Claim>,
  previous: SessionPath[number] | undefined,
): Record<string, string> {
  const claimContent: Record<string, string> = {};
  for (const record of records) {
    const claim = claimById.get(record.id);
    if (!claim || record.tier === "none") continue;
    const body = claim.tiers[record.tier];
    if (body === undefined) continue;
    const rendered = renderClaim(claim.id, record.tier, body);
    const hash = contentHash(rendered);
    record.contentHash = hash;
    if (claim.placement === "epoch-head") continue;
    const priorHash = previousHash(previous, claim.id);
    if (claim.placement === "tail-if-changed" && priorHash === hash) {
      record.reason = "unchanged";
      continue;
    }
    if (claim.placement === "tail-if-changed" && priorHash !== undefined) {
      record.reason = "changed";
    }
    if (priorHash !== hash) claimContent[claim.id] = rendered;
  }
  return claimContent;
}

/**
 * Inside an epoch, a frozen tier-summary decision renders the note the
 * epoch's first manifest logged, so its bytes stay identical.
 */
function attachLoggedSummaries(
  decisions: ChunkDecision[],
  notes: ReadonlyMap<string, string>,
): void {
  for (const decision of decisions) {
    if (decision.tier !== "summary") continue;
    const note = notes.get(decision.id);
    if (note !== undefined) decision.summary = note;
  }
}

/**
 * Builds one manifest row per decision. Each assistant `expand` call on a
 * chunk appends `;expanded_at:<turn>` to its reason; the tier is unchanged.
 */
function chunkRows(
  input: AssembleInput,
  decisions: readonly ChunkDecision[],
): ChunkRow[] {
  const expansions = expandStamps(input.path);
  return decisions.map((decision) => {
    const chunk = input.chunkById.get(decision.id);
    const stamps = expansions.get(decision.id) ?? [];
    const reason = withStamps(decision.reason, stamps);
    if (decision.tier === "none" && chunk?.kind === "tool") {
      return stubRow(chunk, reason, input.count);
    }
    if (decision.tier === "summary" && chunk?.kind === "tool") {
      return summaryRow(chunk, reason, decision.summary ?? "", input.count);
    }
    return keptRow(decision.id, decision.tier, reason, chunk?.tokens ?? 0);
  });
}

/**
 * `decideChunks` counts a summary chunk at its full size; this is the
 * correction to the rendered stub-plus-note size.
 */
function summaryTokenDelta(
  input: AssembleInput,
  decisions: readonly ChunkDecision[],
): number {
  let delta = 0;
  for (const decision of decisions) {
    if (decision.tier !== "summary") continue;
    const chunk = input.chunkById.get(decision.id);
    if (chunk?.kind !== "tool") continue;
    delta +=
      input.count(renderSummary(chunk, decision.summary ?? "")) - chunk.tokens;
  }
  return delta;
}

function frozenRecord(
  claim: Claim,
  parts: Map<string, { tier: Tier; body: string }> | undefined,
  count: TokenCounter,
): ManifestClaim {
  const found = parts?.get(claim.id);
  if (!found || found.tier === "none") {
    return {
      id: claim.id,
      tier: "none",
      reason: "frozen:epoch-head",
      tokens: 0,
      placement: "epoch-head",
      priority: claim.priority,
    };
  }
  return {
    id: claim.id,
    tier: found.tier,
    reason: "frozen:epoch-head",
    tokens: count(found.body),
    contentHash: contentHash(renderClaim(claim.id, found.tier, found.body)),
    placement: "epoch-head",
    priority: claim.priority,
  };
}

function renderEpochHead(
  records: readonly ManifestClaim[],
  claimById: ReadonlyMap<string, Claim>,
): Message | undefined {
  const parts: string[] = [];
  for (const record of records) {
    if (record.placement !== "epoch-head" || record.tier === "none") continue;
    const claim = claimById.get(record.id);
    const body = claim?.tiers[record.tier];
    if (!claim || body === undefined) continue;
    parts.push(renderClaim(claim.id, record.tier, body));
  }
  if (parts.length === 0) return undefined;
  return {
    role: "user",
    content: [{ type: "text", text: parts.join("\n\n") }],
  };
}

function tailBlocks(
  records: readonly ManifestClaim[],
  claimById: ReadonlyMap<string, Claim>,
): { type: "text"; text: string }[] {
  const blocks: { type: "text"; text: string }[] = [];
  const tails = records
    .filter(
      (record) =>
        (record.placement === "tail" ||
          record.placement === "tail-if-changed") &&
        record.tier !== "none",
    )
    .sort(byPriority);
  for (const record of tails) {
    const claim = claimById.get(record.id);
    const tier = record.tier;
    if (!claim || tier === "none") continue;
    const body = claim.tiers[tier];
    if (body === undefined) continue;
    blocks.push({ type: "text", text: renderClaim(claim.id, tier, body) });
  }
  return blocks;
}

function appendTail(
  messages: Message[],
  blocks: { type: "text"; text: string }[],
): void {
  if (blocks.length === 0) return;
  const last = messages.at(-1);
  if (last?.role === "user") {
    last.content.push(...blocks);
    return;
  }
  messages.push({ role: "user", content: blocks });
}

function tokensOf(
  parts: ReadonlyMap<string, { tier: Tier; body: string }>,
  count: TokenCounter,
): number {
  let total = 0;
  for (const part of parts.values()) {
    if (part.tier === "none") continue;
    total += count(part.body);
  }
  return total;
}

function byPriority(left: ManifestClaim, right: ManifestClaim): number {
  return (
    (right.priority ?? 0) - (left.priority ?? 0) ||
    left.id.localeCompare(right.id)
  );
}
