/**
 * Context compiler. One ViewProvider: collect claims, allocate a budget,
 * assemble the view, and attach a manifest.
 *
 * Contract: head is the session-injected head. History is the passthrough
 * projection, including its repair and anomalies, then indexed into chunks:
 * one tool call plus its result, or one prose message. Decisions follow
 * recent, superseded, retry, then oldest-first budget. `keepRecent` defaults
 * to 6. A none tool chunk keeps its tool_use and replaces the tool_result
 * content with a one-line stub. Manifest chunks carry that id, tier, reason,
 * and tokens. A stubbed tool chunk's `tokens` count the stub; `originalTokens`
 * keeps the pre-stub estimate. `budget` also carries `omitted` (none chunks)
 * and `expands` (assistant `expand` tool_use calls already on the path).
 * Each such call appends `;expanded_at:<turn>` to that chunk's reason, where
 * `<turn>` is the preceding manifest's turn. The tier is unchanged.
 * Inside an epoch, chunk decisions from that epoch's first manifest are
 * reused. New chunks go through eliminate, and the recent window counts only
 * those new chunks. The first turn of an epoch has no freeze record.
 * A new epoch opens when `decideEpoch` says rebuild: the view is a full
 * recompile, `epoch` is the previous epoch plus one, and `epochStart.reason`
 * is `cost:<paybackTurns>`, `cost:immediate`, or `context-window-guard`.
 * `route`, when set, runs only on that rebuild. A result sets
 * `epochStart.model` and appends `;route:<reason>` to the decision reason.
 * `scorer` defaults to `ruleScorer` and runs only when this compile rebuilds
 * and at least one kept tool chunk remains. The rule scorer leaves tier and
 * reason untouched. Any other scorer may set a kept tool chunk to none,
 * summary, or full. The chunk reason is the decision's `reason` when that
 * string is non-empty, otherwise `scorer:model`. recent, superseded, retry, and
 * budget reasons win. Summary tool_result text is the stub, a newline, and
 * `summary (model notes, not evidence): `. That string is stored on the chunk
 * record. A fallback appends `;scorer:fallback:<reason>` and keeps the
 * eliminate tiers. When the call reports usage or a fallback, `manifest.scorer`
 * is `{ calls, usage, fallback }`. The score request reuses the previous
 * turn's head and messages rebuilt from the log.
 * Candidates are `listModels` of the current provider. Chunk accesses passed
 * to the policy come from the chunk index and are not copied onto the manifest.
 * The context-window guard is the only rebuild allowed on an epoch's first
 * turn. If that rebuilt view is still over 90% of the context window,
 * `budget.overrun` records the excess when the compile budget itself did
 * not, and a later turn of that epoch does not open another one for the
 * same guard. `expectedTurns` defaults to 20. `minSavingsRatio` defaults
 * to 0.2. Remaining turns are max(1, expectedTurns - turns already started).
 * Cache breakpoints are `head`, the end of the frozen prefix, and the last
 * message. An epoch-head message is rendered once. Later turns of that epoch
 * copy `view_epoch.details.epochHead` when that message is not itself a
 * logged history message, and ignore fresh provider text. Tail blocks are
 * recomputed every turn and appended to the last message when it is a user
 * message. `tail-if-changed` text is stored in `details.claimContent` only
 * when its hash is new. `none` is omitted from the view. A provider that
 * throws becomes one manifest row and does not stop the compile.
 * When `ctx.parentPath` is set, this is a subtask recompile. Messages start
 * with the parent request rebuilt from that path's latest manifest, epoch
 * head included. `frozenDecisions` of the parent's current epoch supplies
 * those chunk tiers. The child's own messages are `own:` chunks and go
 * through `eliminate`. A tail claim `subtask-query` carries `subtask.query`
 * and is appended after that prefix. `decideEpoch` is not called. The
 * decision reason is `subtask:no-rebuild`.
 *
 * Intentionally not in the Log: claim providers, the token counter, the route
 * policy, the scorer function, and the live CompiledView. The kernel persists
 * `view_epoch`, `model_change`, and the manifest, including `scorer` when set.
 */

import type { ClaimRecord, CompiledView, ViewContext } from "@sage/core";
import { passthroughView } from "@sage/core";
import { listModels } from "@sage/provider";
import type {
  Message,
  ModelRef,
  Request,
  Sampling,
  Usage,
} from "@sage/provider/contract";
import { escapeClaimText } from "../claims/escape.js";
import type { RoutePolicy } from "../routing/types.js";
import { ruleScorer } from "../scorer/rule.js";
import type { ScoreDecision, Scorer } from "../scorer/types.js";
import { applyDecisions, renderSummary } from "./apply.js";
import { allocate, defaultTokenCount, type TokenCounter } from "./budget.js";
import { calibrationFor, measuredCacheRead } from "./calibrate.js";
import type { Chunk } from "./chunks.js";
import { indexChunks } from "./chunks.js";
import type { Claim, ClaimProvider, Placement, Tier } from "./claim.js";
import type { EpochDecision, Rates } from "./decide.js";
import { decideEpoch, ratesFor } from "./decide.js";
import type { ChunkDecision } from "./eliminate.js";
import { eliminate } from "./eliminate.js";
import { contentHash, parseClaimMessage, renderClaim } from "./format.js";
import {
  currentEpoch,
  epochOpenReason,
  frozenDecisions,
  frozenSeqCeiling,
} from "./freeze.js";
import { rebuildFromLog } from "./rebuild.js";
import { renderStub } from "./stub.js";

type ManifestClaim = ClaimRecord & {
  contentHash?: string;
  placement?: Placement;
  priority?: number;
};

type ChunkRow = {
  id: string;
  tier: Tier;
  reason: string;
  tokens: number;
  originalTokens?: number;
  summary?: string;
};

type Assembly = {
  messages: Message[];
  records: ManifestClaim[];
  chunkRecords: ChunkRow[];
  used: number;
  claimContent: Record<string, string>;
  epochStart?: { reason: string };
  hasEpochHead: boolean;
};

export function contextCompiler(opts: {
  budget: number;
  claims: ClaimProvider[];
  count?: TokenCounter;
  keepRecent?: number;
  expectedTurns?: number;
  minSavingsRatio?: number;
  route?: RoutePolicy;
  scorer?: Scorer;
  subtask?: { query: string };
}): { view(ctx: ViewContext): Promise<CompiledView> } {
  const count = opts.count ?? defaultTokenCount;
  const keepRecent = opts.keepRecent ?? 6;
  const expectedTurns = opts.expectedTurns ?? 20;
  const minSavingsRatio = opts.minSavingsRatio ?? 0.2;
  const scorer = opts.scorer ?? ruleScorer;

  return {
    async view(ctx: ViewContext): Promise<CompiledView> {
      if (ctx.parentPath) {
        return compileSubtask(ctx, {
          budget: opts.budget,
          query: opts.subtask?.query ?? "",
          count,
          keepRecent,
        });
      }
      const base = await passthroughView.view(ctx);
      const collected = await collect(opts.claims, ctx);
      const seqs = projectionSeqs(ctx.path);
      if (seqs.length !== base.messages.length) {
        throw new Error(
          `projected seqs (${seqs.length}) do not match passthrough messages (${base.messages.length})`,
        );
      }
      const chunks = indexChunks(base.messages, seqs, ctx.tools, count);
      const prior = frozenDecisions(ctx.path, base.epoch);
      const ceiling = frozenSeqCeiling(ctx.path, base.epoch);
      const stay = assemble({
        freezeHead: true,
        prior,
      });
      const fresh = prior
        ? assemble({ freezeHead: false, prior: undefined })
        : stay;
      const calibration = calibrationFor(ctx.path);
      const ratio = calibration?.ratio ?? 1;
      const rates = ratesFor(ctx.prepared.pricing);
      const remainingTurns = Math.max(
        1,
        expectedTurns - Math.max(0, ctx.turn - 1),
      );
      const window = ctx.prepared.contextWindow;
      const guarded =
        epochOpenReason(ctx.path, base.epoch) === "context-window-guard";
      const over = stay.used * ratio > window * 0.9;
      let decision = tagPricing(
        decideEpoch({
          oldTokens: stay.used * ratio,
          newTokens: fresh.used * ratio,
          remainingTurns,
          rates,
          minSavingsRatio,
          contextWindow: window,
          overContext: over && !guarded,
        }),
        rates,
      );
      const guard = decision.reason.includes("context-window-guard");
      if (!prior && !guard && decision.rebuild) {
        decision = {
          ...decision,
          rebuild: false,
          reason: tagReason("savings-too-small", rates),
        };
      }
      let chosen = decision.rebuild && prior ? fresh : stay;
      let scorerReport:
        | { calls: number; usage: Usage; fallback: boolean }
        | undefined;
      if (decision.rebuild) {
        const scored = await runScorer({
          scorer,
          chunks,
          rows: chosen.chunkRecords,
          messages: chosen.messages,
          used: chosen.used,
          count,
          prevRequest: previousRequest(ctx),
          query: latestQuery(ctx.path),
        });
        chosen = {
          ...chosen,
          messages: scored.messages,
          chunkRecords: scored.rows,
          used: scored.used,
        };
        if (scored.fallback) {
          decision = {
            ...decision,
            reason: `${decision.reason};scorer:fallback:${scored.fallback}`,
          };
        }
        scorerReport = scored.report;
      }
      let epoch = base.epoch;
      let epochStart: { reason: string; model?: ModelRef } | undefined =
        chosen.epochStart;
      if (decision.rebuild) {
        epoch = base.epoch + 1;
        epochStart = { reason: openReason(decision) };
        if (opts.route) {
          const routed = opts.route({
            epoch,
            current: ctx.model,
            candidates: [...listModels(ctx.model.provider)],
            viewChunks: routeChunks(chunks, chosen.chunkRecords),
            decision,
            budget: opts.budget,
          });
          if (routed) {
            epochStart = { ...epochStart, model: routed.model };
            decision = {
              ...decision,
              reason: `${decision.reason};route:${routed.reason}`,
            };
          }
        }
      }
      const end = prefixEndIndex(
        chosen.messages.length,
        base.messages.length,
        chosen.hasEpochHead,
        seqs,
        decision.rebuild ? undefined : ceiling,
      );
      const last = Math.max(0, chosen.messages.length - 1);
      const cache = {
        expectedReadTokens: countMessages(chosen.messages, end, count),
        measuredReadTokens: measuredCacheRead(ctx.path),
      };
      let overrun = Math.max(0, chosen.used - opts.budget);
      const scaled = chosen.used * ratio;
      const stillOver = scaled > window * 0.9;
      if (
        overrun === 0 &&
        stillOver &&
        (decision.reason.includes("context-window-guard") || (guarded && over))
      ) {
        overrun = Math.ceil(scaled - window * 0.9);
      }
      const budget = {
        total: opts.budget,
        used: chosen.used,
        overrun,
        omitted: chosen.chunkRecords.filter((record) => record.tier === "none")
          .length,
        expands: countExpands(ctx.path),
      };
      const manifest = {
        epoch,
        turn: ctx.turn,
        budget,
        claims: chosen.records,
        chunks: chosen.chunkRecords,
        anomalies: base.manifest.anomalies,
        epochDecision: decision,
        cache,
        ...(calibration ? { calibration } : {}),
        ...(scorerReport ? { scorer: scorerReport } : {}),
      };
      const view: CompiledView = {
        epoch,
        head: ctx.head,
        messages: chosen.messages,
        cacheBreakpoints: [
          "head",
          { afterMessage: end },
          { afterMessage: last },
        ],
        manifest,
      };
      if (epochStart) view.epochStart = epochStart;
      if (Object.keys(chosen.claimContent).length > 0) {
        view.details = { claimContent: chosen.claimContent };
      }
      return view;

      function assemble(mode: {
        freezeHead: boolean;
        prior: Map<string, ChunkDecision> | undefined;
      }): Assembly {
        const stored = mode.freezeHead
          ? epochHeadFor(ctx.path, base.epoch)
          : undefined;
        const frozen =
          stored && !loggedMessage(ctx.path, stored) ? stored : undefined;
        const frozenParts = frozen ? parseClaimMessage(frozen) : undefined;
        const allocatable: Claim[] = [];
        const frozenClaims: Claim[] = [];
        for (const claim of collected.claims) {
          if (frozen && claim.placement === "epoch-head") {
            frozenClaims.push(claim);
          } else allocatable.push(claim);
        }
        const frozenUsed = frozenParts ? tokensOf(frozenParts, count) : 0;
        const remaining = Math.max(0, opts.budget - frozenUsed);
        const allocation = allocate(allocatable, remaining, count);
        const claimById = new Map(
          allocatable.map((claim) => [claim.id, claim]),
        );
        const records: ManifestClaim[] = [];
        for (const claim of frozenClaims) {
          records.push(frozenRecord(claim, frozenParts, count));
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
        const claimContent: Record<string, string> = {};
        const previous = latestManifest(ctx.path);
        for (const record of records) {
          const claim = claimById.get(record.id);
          if (!claim || record.tier === "none") continue;
          if (claim.placement === "epoch-head") {
            const body = claim.tiers[record.tier];
            if (body === undefined) continue;
            record.contentHash = contentHash(
              renderClaim(claim.id, record.tier, body),
            );
            continue;
          }
          const body = claim.tiers[record.tier];
          if (body === undefined) continue;
          const rendered = renderClaim(claim.id, record.tier, body);
          const hash = contentHash(rendered);
          record.contentHash = hash;
          const priorHash = previousHash(previous, claim.id);
          if (claim.placement === "tail-if-changed" && priorHash === hash) {
            record.reason = "unchanged";
            continue;
          }
          if (
            claim.placement === "tail-if-changed" &&
            priorHash !== undefined
          ) {
            record.reason = "changed";
          }
          if (priorHash !== hash) claimContent[claim.id] = rendered;
        }
        const allocatedUsed = allocation.records.reduce(
          (sum, record) => sum + (record.tokens ?? 0),
          0,
        );
        const claimUsed = frozenUsed + allocatedUsed;
        const decided = decideChunks(
          chunks,
          mode.prior,
          keepRecent,
          Math.max(0, opts.budget - claimUsed),
        );
        if (mode.prior) {
          const notes = loggedSummaries(ctx.path, base.epoch);
          for (const decision of decided.decisions) {
            if (decision.tier !== "summary") continue;
            const note = notes.get(decision.id);
            if (note !== undefined) decision.summary = note;
          }
        }
        const expansions = expandStamps(ctx.path);
        const messages = applyDecisions(
          base.messages,
          chunks,
          decided.decisions,
        );
        const chunkRecords = decided.decisions.map((decision) => {
          const chunk = chunks.find((item) => item.id === decision.id);
          const stamps = expansions.get(decision.id) ?? [];
          const reason = withStamps(decision.reason, stamps);
          const original = chunk?.tokens ?? 0;
          if (decision.tier === "none" && chunk?.kind === "tool") {
            return {
              id: decision.id,
              tier: decision.tier,
              reason,
              tokens: count(renderStub(chunk)),
              originalTokens: original,
            };
          }
          if (decision.tier === "summary" && chunk?.kind === "tool") {
            const summary = decision.summary ?? "";
            return {
              id: decision.id,
              tier: decision.tier,
              reason,
              tokens: count(renderSummary(chunk, summary)),
              originalTokens: original,
              summary,
            };
          }
          return {
            id: decision.id,
            tier: decision.tier,
            reason,
            tokens: original,
          };
        });
        let chunkUsed = decided.used;
        for (const decision of decided.decisions) {
          if (decision.tier !== "summary") continue;
          const chunk = chunks.find((item) => item.id === decision.id);
          if (chunk?.kind !== "tool") continue;
          chunkUsed +=
            count(renderSummary(chunk, decision.summary ?? "")) - chunk.tokens;
        }
        let epochStart: { reason: string } | undefined;
        if (frozen) {
          messages.unshift(frozen);
        } else {
          const head = renderEpochHead(records, claimById);
          if (head) {
            messages.unshift(head);
            epochStart = { reason: "compiler:first-turn" };
          }
        }
        appendTail(messages, tailBlocks(records, claimById));
        return {
          messages,
          records: [...records, ...collected.errors],
          chunkRecords,
          used: claimUsed + chunkUsed,
          claimContent,
          ...(epochStart ? { epochStart } : {}),
          hasEpochHead: frozen !== undefined || epochStart !== undefined,
        };
      }
    },
  };
}

function routeChunks(chunks: readonly Chunk[], records: readonly ChunkRow[]) {
  const byId = new Map(chunks.map((chunk) => [chunk.id, chunk]));
  return records.map((record) => {
    const chunk = byId.get(record.id);
    return {
      id: record.id,
      tier: record.tier,
      reason: record.reason,
      tokens: record.tokens,
      ...(chunk ? { accesses: chunk.accesses } : {}),
    };
  });
}

function decideChunks(
  chunks: Chunk[],
  prior: Map<string, ChunkDecision> | undefined,
  keepRecent: number,
  remainingBudget: number,
): { decisions: ChunkDecision[]; used: number } {
  if (!prior) {
    const eliminated = eliminate(chunks, { keepRecent, remainingBudget });
    return { decisions: eliminated.decisions, used: eliminated.used };
  }
  const fresh = chunks.filter((chunk) => !prior.has(chunk.id));
  let frozenKept = 0;
  for (const chunk of chunks) {
    const kept = prior.get(chunk.id);
    if (!kept || kept.tier === "none") continue;
    frozenKept += chunk.tokens;
  }
  const eliminated = eliminate(fresh, {
    keepRecent,
    remainingBudget: Math.max(0, remainingBudget - frozenKept),
  });
  const freshById = new Map(
    eliminated.decisions.map((decision) => [decision.id, decision]),
  );
  const decisions = chunks.map((chunk) => {
    const kept = prior.get(chunk.id);
    if (kept) return { id: chunk.id, tier: kept.tier, reason: kept.reason };
    return (
      freshById.get(chunk.id) ?? {
        id: chunk.id,
        tier: "full" as const,
        reason: "kept",
      }
    );
  });
  let used = 0;
  for (let index = 0; index < chunks.length; index += 1) {
    if (decisions[index]?.tier === "none") continue;
    used += chunks[index]?.tokens ?? 0;
  }
  return { decisions, used };
}

function withStamps(reason: string, stamps: readonly string[]): string {
  const extra = stamps.filter((stamp) => !reason.split(";").includes(stamp));
  if (extra.length === 0) return reason;
  return `${reason};${extra.join(";")}`;
}

function tagPricing(decision: EpochDecision, rates: Rates): EpochDecision {
  return { ...decision, reason: tagReason(decision.reason, rates) };
}

function tagReason(reason: string, rates: Rates): string {
  if (rates.source !== "fallback" || reason.includes("pricing:fallback")) {
    return reason;
  }
  return `${reason};pricing:fallback`;
}

function openReason(decision: EpochDecision): string {
  if (decision.reason.includes("context-window-guard")) {
    return "context-window-guard";
  }
  if (decision.reason.includes("immediate")) return "cost:immediate";
  if (decision.paybackTurns === null) return "cost:immediate";
  return `cost:${decision.paybackTurns}`;
}

function prefixEndIndex(
  messageCount: number,
  historyCount: number,
  hasEpochHead: boolean,
  seqs: readonly number[],
  ceiling: number | undefined,
): number {
  if (messageCount === 0) return 0;
  const offset = hasEpochHead ? 1 : 0;
  if (historyCount === 0) return Math.min(messageCount - 1, offset);
  if (ceiling === undefined) {
    return Math.min(messageCount - 1, offset + historyCount - 1);
  }
  let lastHistory = -1;
  let prevReal: number | undefined;
  for (let index = 0; index < historyCount; index += 1) {
    const seq = seqs[index] ?? 0;
    const included =
      seq >= 0 ? seq <= ceiling : prevReal !== undefined && prevReal <= ceiling;
    if (seq >= 0) prevReal = seq;
    if (included) lastHistory = index;
  }
  if (lastHistory < 0) return Math.min(messageCount - 1, offset);
  return Math.min(messageCount - 1, offset + lastHistory);
}

function countMessages(
  messages: readonly Message[],
  end: number,
  count: TokenCounter,
): number {
  let total = 0;
  const last = Math.min(end, messages.length - 1);
  for (let index = 0; index <= last; index += 1) {
    const message = messages[index];
    if (message) total += countMessage(message, count);
  }
  return total;
}

function countMessage(message: Message, count: TokenCounter): number {
  let total = 0;
  for (const block of message.content) {
    if (block.type === "text" || block.type === "thinking") {
      total += count(block.text);
      continue;
    }
    if (block.type === "tool_use") {
      const raw =
        block.rawInput.length > 0
          ? block.rawInput
          : (JSON.stringify(block.input ?? "") ?? "");
      total += count(raw);
      continue;
    }
    if (block.type === "tool_result") total += count(block.content);
  }
  return total;
}

function loggedMessage(path: ViewContext["path"], head: Message): boolean {
  const rendered = JSON.stringify(head);
  for (const entry of path) {
    if (entry.kind !== "message") continue;
    if (JSON.stringify(entry.payload) === rendered) return true;
  }
  return false;
}

async function collect(
  providers: readonly ClaimProvider[],
  ctx: ViewContext,
): Promise<{ claims: Claim[]; errors: ManifestClaim[] }> {
  const claims: Claim[] = [];
  const errors: ManifestClaim[] = [];
  const settled = await Promise.all(
    providers.map(async (provider, index) => {
      try {
        return { index, claims: await provider(ctx) };
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        return { index, message };
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

function epochHeadFor(
  path: ViewContext["path"],
  epoch: number,
): Message | undefined {
  let found: Message | undefined;
  for (const entry of path) {
    if (entry.kind !== "view_epoch") continue;
    if (!isRecord(entry.payload) || entry.payload.epoch !== epoch) continue;
    const head = entry.details?.epochHead;
    if (!isMessage(head)) continue;
    found = structuredClone(head);
  }
  return found;
}

function latestManifest(
  path: ViewContext["path"],
): ViewContext["path"][number] | undefined {
  for (let index = path.length - 1; index >= 0; index -= 1) {
    const entry = path[index];
    if (entry?.kind === "manifest") return entry;
  }
  return undefined;
}

function previousHash(
  manifest: ViewContext["path"][number] | undefined,
  id: string,
): string | undefined {
  if (!manifest || !isRecord(manifest.payload)) return undefined;
  const claims = manifest.payload.claims;
  if (!Array.isArray(claims)) return undefined;
  for (const claim of claims) {
    if (!isRecord(claim) || claim.id !== id) continue;
    return typeof claim.contentHash === "string"
      ? claim.contentHash
      : undefined;
  }
  return undefined;
}

function projectionSeqs(path: ViewContext["path"]): number[] {
  const seqs: number[] = [];
  const open: string[] = [];
  const flush = () => {
    if (open.length === 0) return;
    open.length = 0;
    seqs.push(-1);
  };
  for (const entry of path) {
    if (entry.kind !== "message" || !isMessage(entry.payload)) continue;
    if (entry.payload.role === "assistant" && open.length > 0) flush();
    let kept = 0;
    const opened: string[] = [];
    for (const block of entry.payload.content) {
      if (block.type === "tool_result") {
        const index = open.indexOf(block.toolUseId);
        if (index === -1) continue;
        open.splice(index, 1);
        kept += 1;
        continue;
      }
      kept += 1;
      if (block.type === "tool_use") opened.push(block.id);
    }
    if (kept === 0) continue;
    seqs.push(entry.seq);
    open.push(...opened);
  }
  if (open.length > 0) flush();
  return seqs;
}

function expandStamps(path: ViewContext["path"]): Map<string, string[]> {
  const stamps = new Map<string, string[]>();
  let turn = 0;
  for (const entry of path) {
    if (
      entry.kind === "manifest" &&
      isRecord(entry.payload) &&
      typeof entry.payload.turn === "number" &&
      Number.isFinite(entry.payload.turn)
    ) {
      turn = entry.payload.turn;
    }
    if (entry.kind !== "message" || !isMessage(entry.payload)) continue;
    if (entry.payload.role !== "assistant") continue;
    for (const block of entry.payload.content) {
      if (block.type !== "tool_use" || block.name !== "expand") continue;
      const id = expandTarget(block.input);
      if (!id) continue;
      const list = stamps.get(id) ?? [];
      list.push(`expanded_at:${turn}`);
      stamps.set(id, list);
    }
  }
  return stamps;
}

function countExpands(path: ViewContext["path"]): number {
  let total = 0;
  for (const entry of path) {
    if (entry.kind !== "message" || !isMessage(entry.payload)) continue;
    if (entry.payload.role !== "assistant") continue;
    for (const block of entry.payload.content) {
      if (block.type === "tool_use" && block.name === "expand") total += 1;
    }
  }
  return total;
}

function expandTarget(input: unknown): string | undefined {
  if (!isRecord(input)) return undefined;
  if (typeof input.id !== "string" || input.id.length === 0) return undefined;
  return input.id;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isMessage(value: unknown): value is Message {
  if (!isRecord(value)) return false;
  return (
    (value.role === "user" || value.role === "assistant") &&
    Array.isArray(value.content)
  );
}

function compileSubtask(
  ctx: ViewContext,
  opts: {
    budget: number;
    query: string;
    count: TokenCounter;
    keepRecent: number;
  },
): CompiledView {
  const parentPath = ctx.parentPath ?? [];
  const epoch = currentEpoch(parentPath);
  const manifest = latestManifest(parentPath);
  const turn = manifestTurn(manifest);
  if (turn === undefined) {
    throw new Error("subtask parent path has no manifest");
  }
  const rebuilt = rebuildFromLog(parentPath, turn);
  const parentMessages = structuredClone(rebuilt.messages);
  const parentRows = overlayFrozen(
    chunkRows(manifest),
    frozenDecisions(parentPath, epoch),
  );
  const projected = passthroughView.view(ctx);
  if (projected instanceof Promise) {
    throw new Error("passthrough projection must be synchronous");
  }
  const seqs = projectionSeqs(ctx.path);
  if (seqs.length !== projected.messages.length) {
    throw new Error(
      `projected seqs (${seqs.length}) do not match passthrough messages (${projected.messages.length})`,
    );
  }
  const ownChunks = indexChunks(
    projected.messages,
    seqs,
    ctx.tools,
    opts.count,
  ).map((chunk) => ({ ...chunk, id: `own:${chunk.id}` }));
  const rendered = renderClaim(
    "subtask-query",
    "full",
    escapeClaimText(opts.query),
  );
  const claimTokens = opts.count(rendered);
  const parentTokens = keptTokens(parentRows);
  const eliminated = eliminate(ownChunks, {
    keepRecent: opts.keepRecent,
    remainingBudget: Math.max(0, opts.budget - parentTokens - claimTokens),
  });
  const ownMessages = applyDecisions(
    projected.messages,
    ownChunks,
    eliminated.decisions,
  );
  const parentCount = parentMessages.length;
  const messages = parentMessages;
  messages.push(...ownMessages);
  appendSubtaskQuery(messages, parentCount, rendered);
  const ownRows = ownRowsFrom(ownChunks, eliminated.decisions, opts.count);
  const used = parentTokens + eliminated.used + claimTokens;
  const omitted =
    parentRows.filter((row) => row.tier === "none").length +
    ownRows.filter((row) => row.tier === "none").length;
  const prefixEnd = Math.max(0, parentCount - 1);
  const last = Math.max(0, messages.length - 1);
  const budget = {
    total: opts.budget,
    used,
    overrun: Math.max(0, used - opts.budget),
    omitted,
    expands: countExpands(ctx.path),
  };
  const claims: ManifestClaim[] = [
    {
      id: "subtask-query",
      tier: "full",
      reason: "pinned",
      tokens: claimTokens,
      placement: "tail",
      priority: 100,
      contentHash: contentHash(rendered),
    },
  ];
  const view: CompiledView = {
    epoch,
    head: ctx.head,
    messages,
    cacheBreakpoints: [
      "head",
      { afterMessage: prefixEnd },
      { afterMessage: last },
    ],
    manifest: {
      epoch,
      turn: ctx.turn,
      budget,
      claims,
      chunks: [...parentRows, ...ownRows],
      anomalies: projected.manifest.anomalies,
      epochDecision: {
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
      },
      cache: {
        expectedReadTokens: parentTokens,
        measuredReadTokens: measuredCacheRead(parentPath),
      },
    },
    details: { claimContent: { "subtask-query": rendered } },
  };
  return view;
}

function manifestTurn(
  entry: ViewContext["path"][number] | undefined,
): number | undefined {
  if (!entry || !isRecord(entry.payload)) return undefined;
  return typeof entry.payload.turn === "number"
    ? entry.payload.turn
    : undefined;
}

function chunkRows(entry: ViewContext["path"][number] | undefined): ChunkRow[] {
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
    if (!isTierName(value.tier)) continue;
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
  return decisions.map((decision) => {
    const chunk = chunks.find((item) => item.id === decision.id);
    const original = chunk?.tokens ?? 0;
    if (decision.tier === "none" && chunk?.kind === "tool") {
      return {
        id: decision.id,
        tier: decision.tier,
        reason: decision.reason,
        tokens: count(renderStub(chunk)),
        originalTokens: original,
      };
    }
    return {
      id: decision.id,
      tier: decision.tier,
      reason: decision.reason,
      tokens: original,
    };
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

function isTierName(value: unknown): value is Tier {
  return (
    value === "none" ||
    value === "line" ||
    value === "summary" ||
    value === "full"
  );
}

const SCORE_TIERS = new Set<Tier>(["none", "summary", "full"]);
const SUMMARY_LIMIT = 200;
const EMPTY_USAGE: Usage = {
  inputOther: 0,
  inputCacheRead: 0,
  inputCacheCreation: 0,
  output: 0,
};

async function runScorer(input: {
  scorer: Scorer;
  chunks: Chunk[];
  rows: ChunkRow[];
  messages: Message[];
  used: number;
  count: TokenCounter;
  prevRequest: Request | undefined;
  query: string;
}): Promise<{
  rows: ChunkRow[];
  messages: Message[];
  used: number;
  fallback?: string;
  report?: { calls: number; usage: Usage; fallback: boolean };
}> {
  const candidates = keptToolChunks(input.rows, input.chunks);
  if (candidates.length === 0) {
    return { rows: input.rows, messages: input.messages, used: input.used };
  }
  let result: {
    decisions: ScoreDecision[];
    usage?: Usage;
    fallback?: string;
  };
  try {
    result = await input.scorer.score({
      candidates,
      prevRequest: input.prevRequest,
      query: input.query,
    });
  } catch {
    result = { decisions: [], fallback: "error" };
  }
  const report = reportOf(result);
  if (result.fallback !== undefined || input.scorer.name === "rule") {
    return {
      rows: input.rows,
      messages: input.messages,
      used: input.used,
      ...(result.fallback !== undefined ? { fallback: result.fallback } : {}),
      ...(report ? { report } : {}),
    };
  }
  const merged = mergeScore(
    input.rows,
    input.chunks,
    result.decisions,
    input.count,
  );
  const decisions = merged.rows.map((row) => ({
    id: row.id,
    tier: row.tier,
    reason: row.reason,
    ...(row.summary !== undefined ? { summary: row.summary } : {}),
  }));
  return {
    rows: merged.rows,
    messages: applyDecisions(input.messages, input.chunks, decisions),
    used: input.used + merged.usedDelta,
    ...(report ? { report } : {}),
  };
}

function keptToolChunks(
  rows: readonly ChunkRow[],
  chunks: readonly Chunk[],
): Chunk[] {
  const byId = new Map(chunks.map((chunk) => [chunk.id, chunk]));
  const kept: Chunk[] = [];
  for (const row of rows) {
    if (rootReason(row.reason) !== "kept") continue;
    const chunk = byId.get(row.id);
    if (chunk?.kind !== "tool") continue;
    kept.push(chunk);
  }
  return kept;
}

function mergeScore(
  rows: readonly ChunkRow[],
  chunks: readonly Chunk[],
  decisions: readonly ScoreDecision[],
  count: TokenCounter,
): { rows: ChunkRow[]; usedDelta: number } {
  const byId = new Map(decisions.map((decision) => [decision.id, decision]));
  const chunksById = new Map(chunks.map((chunk) => [chunk.id, chunk]));
  let usedDelta = 0;
  const next = rows.map((row) => {
    if (rootReason(row.reason) !== "kept") return row;
    const chunk = chunksById.get(row.id);
    if (chunk?.kind !== "tool") return row;
    const scored = byId.get(row.id);
    if (!scored || !SCORE_TIERS.has(scored.tier)) return row;
    const reason = modelReason(row.reason, scored);
    if (scored.tier === "full") {
      return {
        id: row.id,
        tier: "full" as const,
        reason,
        tokens: chunk.tokens,
      };
    }
    if (scored.tier === "none") {
      usedDelta -= chunk.tokens;
      return {
        id: row.id,
        tier: "none" as const,
        reason,
        tokens: count(renderStub(chunk)),
        originalTokens: chunk.tokens,
      };
    }
    const summary = clampSummary(scored.summary ?? "");
    const rendered = renderSummary(chunk, summary);
    usedDelta += count(rendered) - chunk.tokens;
    return {
      id: row.id,
      tier: "summary" as const,
      reason,
      tokens: count(rendered),
      originalTokens: chunk.tokens,
      summary,
    };
  });
  return { rows: next, usedDelta };
}

function reportOf(result: {
  usage?: Usage;
  fallback?: string;
}): { calls: number; usage: Usage; fallback: boolean } | undefined {
  if (result.usage === undefined && result.fallback === undefined)
    return undefined;
  const preCall =
    result.fallback === "no-previous-request" ||
    result.fallback === "no-adapter";
  return {
    calls: preCall ? 0 : 1,
    usage: result.usage ?? EMPTY_USAGE,
    fallback: result.fallback !== undefined,
  };
}

function rootReason(reason: string): string {
  return reason.split(";")[0] ?? reason;
}

function modelReason(reason: string, decision: ScoreDecision): string {
  const root =
    decision.reason !== undefined && decision.reason.length > 0
      ? decision.reason
      : "scorer:model";
  const extra = reason
    .split(";")
    .slice(1)
    .filter((part) => part.length > 0);
  if (extra.length === 0) return root;
  return `${root};${extra.join(";")}`;
}

function clampSummary(summary: string): string {
  if (summary.length <= SUMMARY_LIMIT) return summary;
  return summary.slice(0, SUMMARY_LIMIT);
}

function previousRequest(ctx: ViewContext): Request | undefined {
  const manifest = latestManifest(ctx.path);
  const turn = manifestTurn(manifest);
  if (!manifest || turn === undefined) return undefined;
  const header = latestHeader(ctx.path);
  if (!header) return undefined;
  try {
    const rebuilt = rebuildFromLog(ctx.path, turn);
    const last = Math.max(0, rebuilt.messages.length - 1);
    return {
      model: { provider: header.provider, model: header.model },
      head: rebuilt.head,
      messages: rebuilt.messages,
      cacheBreakpoints: ["head", { afterMessage: last }],
      sampling: header.sampling,
    };
  } catch {
    return undefined;
  }
}

function latestHeader(
  path: ViewContext["path"],
): { provider: string; model: string; sampling: Sampling } | undefined {
  for (let index = path.length - 1; index >= 0; index -= 1) {
    const entry = path[index];
    if (entry?.kind !== "request_header" || !isRecord(entry.payload)) continue;
    const provider = entry.payload.provider;
    const model = entry.payload.model;
    if (typeof provider !== "string" || typeof model !== "string") continue;
    return { provider, model, sampling: samplingOf(entry.payload.sampling) };
  }
  return undefined;
}

function samplingOf(value: unknown): Sampling {
  if (!isRecord(value)) return {};
  const sampling: Sampling = {};
  if (typeof value.maxTokens === "number") sampling.maxTokens = value.maxTokens;
  if (typeof value.temperature === "number")
    sampling.temperature = value.temperature;
  return sampling;
}

function latestQuery(path: ViewContext["path"]): string {
  for (let index = path.length - 1; index >= 0; index -= 1) {
    const entry = path[index];
    if (entry?.kind !== "message" || !isMessage(entry.payload)) continue;
    if (entry.payload.role !== "user") continue;
    const text = entry.payload.content
      .filter((block) => block.type === "text")
      .map((block) => block.text)
      .join("\n");
    if (text.length > 0) return text;
  }
  return "";
}

function loggedSummaries(
  path: ViewContext["path"],
  epoch: number,
): Map<string, string> {
  const entry = epochManifest(path, epoch);
  const notes = new Map<string, string>();
  if (
    !entry ||
    !isRecord(entry.payload) ||
    !Array.isArray(entry.payload.chunks)
  ) {
    return notes;
  }
  for (const chunk of entry.payload.chunks) {
    if (!isRecord(chunk) || typeof chunk.id !== "string") continue;
    if (chunk.tier !== "summary" || typeof chunk.summary !== "string") continue;
    notes.set(chunk.id, chunk.summary);
  }
  return notes;
}

function epochManifest(
  path: ViewContext["path"],
  epoch: number,
): ViewContext["path"][number] | undefined {
  let start = -1;
  let opened = false;
  for (let index = 0; index < path.length; index += 1) {
    const entry = path[index];
    if (entry?.kind !== "view_epoch" || !isRecord(entry.payload)) continue;
    if (entry.payload.epoch !== epoch) continue;
    start = index;
    opened = true;
    break;
  }
  if (!opened && epoch !== 0) return undefined;
  for (let index = start + 1; index < path.length; index += 1) {
    const entry = path[index];
    if (!entry) continue;
    if (entry.kind === "view_epoch") break;
    if (entry.kind === "manifest") return entry;
  }
  return undefined;
}
