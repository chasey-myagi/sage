/**
 * Context compiler. One ViewProvider: collect claims, allocate a budget,
 * assemble the view, and attach a manifest.
 *
 * Contract: head is the session-injected head. History is the passthrough
 * projection, including its repair and anomalies. Epoch stays whatever the
 * path already says; this compiler never opens a new epoch on its own.
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
import { allocate, defaultTokenCount, type TokenCounter } from "./budget.js";
import type { Claim, ClaimProvider, Placement, Tier } from "./claim.js";
import { contentHash, parseClaimMessage, renderClaim } from "./format.js";

type ManifestClaim = ClaimRecord & {
  contentHash?: string;
  placement?: Placement;
  priority?: number;
};

export function contextCompiler(opts: {
  budget: number;
  claims: ClaimProvider[];
  count?: TokenCounter;
}): { view(ctx: ViewContext): Promise<CompiledView> } {
  const count = opts.count ?? defaultTokenCount;

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

      const messages = base.messages.map((message) => ({
        role: message.role,
        content: [...message.content],
      }));
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

      const allocatedUsed = allocation.records.reduce(
        (sum, record) => sum + (record.tokens ?? 0),
        0,
      );
      const used = frozenUsed + allocatedUsed;
      const view: CompiledView = {
        epoch: base.epoch,
        head: ctx.head,
        messages,
        cacheBreakpoints: [...base.cacheBreakpoints],
        manifest: {
          epoch: base.epoch,
          turn: ctx.turn,
          budget: {
            total: opts.budget,
            used,
            overrun: Math.max(0, used - opts.budget),
          },
          claims: [...records, ...collected.errors],
          chunks: base.manifest.chunks,
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
