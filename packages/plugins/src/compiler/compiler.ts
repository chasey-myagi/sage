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
 * Epoch stays whatever the path already says; this compiler never opens a
 * new epoch on its own.
 * An epoch-head message is rendered once. Later turns of that epoch copy
 * `view_epoch.details.epochHead` and ignore fresh provider text. Tail blocks
 * are recomputed every turn and appended to the last message when it is a
 * user message. `tail-if-changed` text is stored in `details.claimContent`
 * only when its hash is new. `none` is omitted from the view. A provider
 * that throws becomes one manifest row and does not stop the compile.
 *
 * Intentionally not in the Log: claim providers, the token counter, and the
 * live CompiledView. The kernel persists `view_epoch` and the manifest.
 */

import type { ClaimRecord, CompiledView, ViewContext } from "@sage/core";
import { passthroughView } from "@sage/core";
import type { Message } from "@sage/provider/contract";
import { applyDecisions } from "./apply.js";
import { allocate, defaultTokenCount, type TokenCounter } from "./budget.js";
import { indexChunks } from "./chunks.js";
import type { Claim, ClaimProvider, Placement, Tier } from "./claim.js";
import { eliminate } from "./eliminate.js";
import { contentHash, parseClaimMessage, renderClaim } from "./format.js";
import { renderStub } from "./stub.js";

type ManifestClaim = ClaimRecord & {
  contentHash?: string;
  placement?: Placement;
  priority?: number;
};

export function contextCompiler(opts: {
  budget: number;
  claims: ClaimProvider[];
  count?: TokenCounter;
  keepRecent?: number;
}): { view(ctx: ViewContext): Promise<CompiledView> } {
  const count = opts.count ?? defaultTokenCount;
  const keepRecent = opts.keepRecent ?? 6;

  return {
    async view(ctx: ViewContext): Promise<CompiledView> {
      const base = await passthroughView.view(ctx);
      const collected = await collect(opts.claims, ctx);
      const frozen = epochHeadFor(ctx.path, base.epoch);
      const frozenParts = frozen ? parseClaimMessage(frozen) : undefined;

      const allocatable: Claim[] = [];
      const frozenClaims: Claim[] = [];
      for (const claim of collected.claims) {
        if (frozen && claim.placement === "epoch-head")
          frozenClaims.push(claim);
        else allocatable.push(claim);
      }

      const frozenUsed = frozenParts ? tokensOf(frozenParts, count) : 0;
      const remaining = Math.max(0, opts.budget - frozenUsed);
      const allocation = allocate(allocatable, remaining, count);
      const claimById = new Map(allocatable.map((claim) => [claim.id, claim]));

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
        const prior = previousHash(previous, claim.id);
        if (claim.placement === "tail-if-changed" && prior === hash) {
          record.reason = "unchanged";
          continue;
        }
        if (claim.placement === "tail-if-changed" && prior !== undefined) {
          record.reason = "changed";
        }
        if (prior !== hash) claimContent[claim.id] = rendered;
      }

      const allocatedUsed = allocation.records.reduce(
        (sum, record) => sum + (record.tokens ?? 0),
        0,
      );
      const claimUsed = frozenUsed + allocatedUsed;
      const seqs = projectionSeqs(ctx.path);
      if (seqs.length !== base.messages.length) {
        throw new Error(
          `projected seqs (${seqs.length}) do not match passthrough messages (${base.messages.length})`,
        );
      }
      const chunks = indexChunks(base.messages, seqs, ctx.tools, count);
      const eliminated = eliminate(chunks, {
        keepRecent,
        remainingBudget: Math.max(0, opts.budget - claimUsed),
      });
      const used = claimUsed + eliminated.used;
      const expansions = expandStamps(ctx.path);
      const messages = applyDecisions(
        base.messages,
        chunks,
        eliminated.decisions,
      );
      const chunkRecords = eliminated.decisions.map((decision) => {
        const chunk = chunks.find((item) => item.id === decision.id);
        const stamps = expansions.get(decision.id) ?? [];
        const reason =
          stamps.length > 0
            ? `${decision.reason};${stamps.join(";")}`
            : decision.reason;
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
        return {
          id: decision.id,
          tier: decision.tier,
          reason,
          tokens: original,
        };
      });
      const budget = {
        total: opts.budget,
        used,
        overrun: Math.max(0, used - opts.budget),
        omitted: eliminated.decisions.filter(
          (decision) => decision.tier === "none",
        ).length,
        expands: countExpands(ctx.path),
      };
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
      const view: CompiledView = {
        epoch: base.epoch,
        head: ctx.head,
        messages,
        cacheBreakpoints: [...base.cacheBreakpoints],
        manifest: {
          epoch: base.epoch,
          turn: ctx.turn,
          budget,
          claims: [...records, ...collected.errors],
          chunks: chunkRecords,
          anomalies: base.manifest.anomalies,
        },
      };
      if (epochStart) view.epochStart = epochStart;
      if (Object.keys(claimContent).length > 0) {
        view.details = { claimContent };
      }
      return view;
    },
  };
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
