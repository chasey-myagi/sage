/**
 * `sage stats --turns`: a per-turn audit of one session path, for reading
 * how the compiler behaved turn by turn.
 *
 * Contract: one row per model call that got an assistant reply (a
 * request_header without a reply yields none), then path-wide invariants.
 * Rows hold counts, chunk and tool_use ids, tool names, tiers, and the
 * epoch decision's numeric fields. Usage and the hit ratio use the
 * `runHealth` definitions. No `reason` string is copied: chunk and
 * decision reasons can carry paths or provider text (a routing reason
 * names the sensitive file it matched).
 *
 * Intentionally not in the Log: the report. It is computed from the Log
 * and nothing reads it back. Message text, reasons, tool inputs, and
 * paths are left out of the report itself.
 */

import {
  finiteOr,
  type HealthTokens,
  isFiniteNumber,
  isMessage,
  isRecord,
  isTier,
  type Manifest,
  type RunHealth,
  requestHeaderOf,
  runHealth,
  type StoredEntry,
  type Tier,
} from "@sage/core";
import type { ModelRef } from "@sage/provider/contract";

/** A chunk whose tier changed between two turns of the same epoch. */
export type TierFlip = { id: string; from: Tier; to: Tier };

/** One turn of the audit. Manifest fields are null when it had none. */
export type TurnRow = {
  /** The manifest's `turn`, else the 1-based row position. */
  turn: number;
  epoch: number | null;
  model: ModelRef;
  /** This turn's assistant usage, in `runHealth` naming. */
  tokens: HealthTokens;
  /** `runHealth.cacheHitRatio` of this turn alone. */
  cacheHitRatio: number | null;
  budget: { used: number; omitted: number; expands: number } | null;
  /** Manifest chunks per tier. */
  tiers: Record<Tier, number> | null;
  /** Tier changes against the previous row, only when both are one epoch. */
  flips: TierFlip[];
  /** The decision's structured fields; its `reason` text is left out. */
  epochDecision: Pick<
    NonNullable<Manifest["epochDecision"]>,
    "rebuild" | "oldTokens" | "newTokens" | "remainingTurns" | "paybackTurns"
  > | null;
  /** tool_use names of the assistant reply, in order. */
  tools: string[];
};

/** Path-wide checks reported after the rows. */
export type TurnInvariants = {
  toolUses: number;
  toolResults: number;
  /** tool_use ids with no tool_result anywhere on the path. */
  unpairedToolUses: string[];
  /** tool_result ids with no tool_use anywhere on the path. */
  orphanToolResults: string[];
  /** tool_result blocks with `isError: true`. */
  toolErrors: number;
  requestHeaders: number;
  manifests: number;
  /** Distinct request_header models, in first-seen order. */
  models: ModelRef[];
  /** The last terminal `outcome`, or null when the run never ended. */
  outcome: string | null;
  health: RunHealth;
};

/** What `sage stats --turns` prints. */
export type TurnReport = { turns: TurnRow[]; invariants: TurnInvariants };

type ManifestView = {
  epoch: number;
  turn: number;
  chunks: { id: string; tier: Tier }[];
  budget: TurnRow["budget"];
  epochDecision: TurnRow["epochDecision"];
};

type OpenTurn = { model: ModelRef; manifest?: ManifestView };

/** Builds the per-turn audit of `path`. Pure; never throws on bad entries. */
export function turnReport(path: readonly StoredEntry[]): TurnReport {
  const turns: TurnRow[] = [];
  let open: OpenTurn | undefined;
  let previous: ManifestView | undefined;
  for (const entry of path) {
    const header = requestHeaderOf(entry);
    if (header) {
      open = { model: { provider: header.provider, model: header.model } };
      continue;
    }
    if (!open) continue;
    if (entry.kind === "manifest") {
      const manifest = readManifest(entry.payload);
      if (manifest) open.manifest = manifest;
      continue;
    }
    if (!isAssistantEntry(entry)) continue;
    turns.push(turnRow(turns.length + 1, open, entry, previous));
    previous = open.manifest ?? previous;
    open = undefined;
  }
  return { turns, invariants: invariantsOf(path) };
}

function turnRow(
  position: number,
  open: OpenTurn,
  assistant: StoredEntry,
  previous: ManifestView | undefined,
): TurnRow {
  // runHealth over the reply alone gives this turn's usage and hit ratio
  // with exactly the run-level definitions.
  const health = runHealth([assistant]);
  const manifest = open.manifest;
  return {
    turn: manifest?.turn ?? position,
    epoch: manifest?.epoch ?? null,
    model: open.model,
    tokens: health.tokens,
    cacheHitRatio: health.cacheHitRatio,
    budget: manifest?.budget ?? null,
    tiers: manifest ? tierCounts(manifest) : null,
    flips: manifest && previous ? tierFlips(previous, manifest) : [],
    epochDecision: manifest?.epochDecision ?? null,
    tools: toolUses(assistant).map((use) => use.name),
  };
}

function tierCounts(manifest: ManifestView): Record<Tier, number> {
  const counts: Record<Tier, number> = {
    full: 0,
    summary: 0,
    line: 0,
    none: 0,
  };
  for (const chunk of manifest.chunks) counts[chunk.tier] += 1;
  return counts;
}

function tierFlips(before: ManifestView, after: ManifestView): TierFlip[] {
  if (before.epoch !== after.epoch) return [];
  const earlier = new Map(before.chunks.map((chunk) => [chunk.id, chunk.tier]));
  const flips: TierFlip[] = [];
  for (const chunk of after.chunks) {
    const from = earlier.get(chunk.id);
    if (from !== undefined && from !== chunk.tier) {
      flips.push({ id: chunk.id, from, to: chunk.tier });
    }
  }
  return flips;
}

/**
 * Reads the fields the audit uses, guarding each one; a malformed chunk
 * line is skipped and a malformed budget or decision reads as null.
 */
function readManifest(payload: unknown): ManifestView | undefined {
  if (!isRecord(payload) || !Array.isArray(payload.chunks)) return undefined;
  if (!isFiniteNumber(payload.epoch) || !isFiniteNumber(payload.turn)) {
    return undefined;
  }
  return {
    epoch: payload.epoch,
    turn: payload.turn,
    chunks: payload.chunks.flatMap((chunk) =>
      isRecord(chunk) && typeof chunk.id === "string" && isTier(chunk.tier)
        ? [{ id: chunk.id, tier: chunk.tier }]
        : [],
    ),
    budget: readBudget(payload.budget),
    epochDecision: readDecision(payload.epochDecision),
  };
}

function readBudget(value: unknown): TurnRow["budget"] {
  if (!isRecord(value)) return null;
  return {
    used: finiteOr(value.used, 0),
    omitted: finiteOr(value.omitted, 0),
    expands: finiteOr(value.expands, 0),
  };
}

/** A missing or non-finite `paybackTurns` reads as null (no savings). */
function readDecision(value: unknown): TurnRow["epochDecision"] {
  if (!isRecord(value) || typeof value.rebuild !== "boolean") return null;
  const { oldTokens, newTokens, remainingTurns } = value;
  if (!isFiniteNumber(oldTokens) || !isFiniteNumber(newTokens)) return null;
  if (!isFiniteNumber(remainingTurns)) return null;
  return {
    rebuild: value.rebuild,
    oldTokens,
    newTokens,
    remainingTurns,
    paybackTurns: isFiniteNumber(value.paybackTurns)
      ? value.paybackTurns
      : null,
  };
}

function invariantsOf(path: readonly StoredEntry[]): TurnInvariants {
  const useIds: string[] = [];
  const resultIds: string[] = [];
  const models: ModelRef[] = [];
  let requestHeaders = 0;
  let manifests = 0;
  let outcome: string | null = null;
  for (const entry of path) {
    const header = requestHeaderOf(entry);
    if (header) {
      requestHeaders += 1;
      addModel(models, { provider: header.provider, model: header.model });
    }
    if (entry.kind === "manifest") manifests += 1;
    if (entry.kind === "terminal" && isRecord(entry.payload)) {
      const value = entry.payload.outcome;
      if (typeof value === "string") outcome = value;
    }
    if (isAssistantEntry(entry)) {
      useIds.push(...toolUses(entry).map((use) => use.id));
    }
    resultIds.push(...toolResultIds(entry));
  }
  const health = runHealth(path);
  const uses = new Set(useIds);
  const results = new Set(resultIds);
  return {
    toolUses: useIds.length,
    toolResults: resultIds.length,
    unpairedToolUses: useIds.filter((id) => !results.has(id)),
    orphanToolResults: resultIds.filter((id) => !uses.has(id)),
    toolErrors: health.toolErrors,
    requestHeaders,
    manifests,
    models,
    outcome,
    health,
  };
}

function addModel(models: ModelRef[], model: ModelRef): void {
  const seen = models.some(
    (known) => known.provider === model.provider && known.model === model.model,
  );
  if (!seen) models.push(model);
}

function isAssistantEntry(entry: StoredEntry): boolean {
  return (
    entry.kind === "message" &&
    isMessage(entry.payload) &&
    entry.payload.role === "assistant"
  );
}

function toolUses(entry: StoredEntry): { id: string; name: string }[] {
  if (!isMessage(entry.payload)) return [];
  return entry.payload.content.flatMap((block) =>
    isRecord(block) &&
    block.type === "tool_use" &&
    typeof block.id === "string" &&
    typeof block.name === "string"
      ? [{ id: block.id, name: block.name }]
      : [],
  );
}

function toolResultIds(entry: StoredEntry): string[] {
  if (entry.kind !== "message" || !isMessage(entry.payload)) return [];
  if (entry.payload.role !== "user") return [];
  return entry.payload.content.flatMap((block) =>
    isRecord(block) &&
    block.type === "tool_result" &&
    typeof block.toolUseId === "string"
      ? [block.toolUseId]
      : [],
  );
}
