/**
 * Context compiler: the plugin's only ViewProvider. Each turn it collects
 * claims, decides a tier for every history chunk, and returns the view with
 * a manifest that records every decision.
 *
 * Contract: `head` is always the session-injected head. Inside an epoch the
 * rendered prefix is byte-identical to the previous turn's, so the provider
 * prefix cache keeps hitting; only the tail changes. A new epoch opens only
 * when the cost check or the context-window guard says rebuild. Every
 * model-visible byte is reproducible from the Log: the manifest, the
 * `view_epoch` head, and `details.claimContent`. A claim provider that
 * throws becomes a manifest row; a scorer that throws or falls back
 * degrades to the deterministic tiers. Only kernel defects throw.
 *
 * Stages live in sibling files: `assemble.ts` (claims and chunk rows),
 * `chunk-decisions.ts` (freeze), `epoch.ts` (cost check), `score-*.ts`
 * (scorers), `breakpoints.ts`, and `subtask-view.ts`.
 *
 * Intentionally not in the Log: claim providers, the token counter, the
 * route policy, the scorer and shadow scorer functions, and the live
 * CompiledView. The kernel persists `view_epoch`, `model_change`, and the
 * manifest, including `scorer`, `entryScored`, and `shadow` when set.
 */

import type { CompiledView, EntryScoredReport, ViewContext } from "@sage/core";
import { passthroughView } from "@sage/core";
import type { RoutePolicy } from "../routing/types.js";
import { ruleScorer } from "../scorer/rule.js";
import type { Scorer } from "../scorer/types.js";
import { type Assembly, assembleView, collectClaims } from "./assemble.js";
import { cacheBreakpointsFor } from "./breakpoints.js";
import { defaultTokenCount, type TokenCounter } from "./budget.js";
import {
  calibrationFor,
  expectedFrozenPrefix,
  measuredCacheRead,
} from "./calibrate.js";
import { type FrozenMap, freezeAccounting } from "./chunk-decisions.js";
import type { Chunk } from "./chunks.js";
import { indexChunks } from "./chunks.js";
import type { ClaimProvider } from "./claim.js";
import { ratesFor } from "./decide.js";
import {
  appendReason,
  chooseEpoch,
  DEFAULT_ALPHA,
  DEFAULT_EXPECTED_TURNS,
  DEFAULT_MIN_SAVINGS_RATIO,
  type EpochChoice,
  openEpoch,
  type Projection,
  projectionFor,
  remainingTurnsFor,
  viewOverrun,
} from "./epoch.js";
import {
  epochOpenReason,
  frozenDecisions,
  frozenSeqCeiling,
} from "./freeze.js";
import {
  assertSeqsMatch,
  countExpands,
  latestQuery,
  previousRequest,
  projectionSeqs,
} from "./log-read.js";
import { firstById } from "./rows.js";
import { requestFromCompiled } from "./score-common.js";
import { resolveMinScoreTokens, runEntryScorer } from "./score-entry.js";
import {
  type RebuildScorerReport,
  runRebuildScorer,
  type ScoredView,
} from "./score-rebuild.js";
import { runShadowScorer, type ShadowBody } from "./score-shadow.js";
import { compileSubtask } from "./subtask-view.js";

/**
 * Default `keepRecent`: how many of the latest tool chunks stay full with
 * reason `recent` regardless of budget.
 */
export const DEFAULT_KEEP_RECENT = 6;

/** Options of `contextCompiler`. Token counts are estimates from `count`. */
export type ContextCompilerOptions = {
  /** Token budget for claims plus history chunks. */
  budget: number;
  claims: ClaimProvider[];
  /** Token estimator. Defaults to `defaultTokenCount`. */
  count?: TokenCounter;
  /** Defaults to `DEFAULT_KEEP_RECENT` (6). */
  keepRecent?: number;
  /** Defaults to `DEFAULT_EXPECTED_TURNS` (20). */
  expectedTurns?: number;
  /** Defaults to `DEFAULT_ALPHA` (0.5). */
  alpha?: number;
  /** Defaults to `DEFAULT_MIN_SAVINGS_RATIO` (0.2). */
  minSavingsRatio?: number;
  /**
   * Picks a model when an epoch opens; runs only on a rebuild. Candidates
   * are `listModels` of the current provider.
   */
  route?: RoutePolicy;
  /** Defaults to `ruleScorer`, which changes no tier or reason. */
  scorer?: Scorer;
  /** Records what this scorer would decide; never applied. */
  shadowScorer?: Scorer;
  /**
   * `rebuild` (default) scores kept tool chunks when an epoch opens.
   * `entry` scores each new tool chunk once, on the turn it enters the
   * view, and runs neither the rebuild scorer nor `shadowScorer`.
   */
  scorerApply?: "rebuild" | "entry";
  /**
   * Entry mode only: smallest tool chunk worth scoring. Defaults to
   * `DEFAULT_MIN_SCORE_TOKENS` (300); a negative or non-finite value also
   * falls back to it.
   */
  minScoreTokens?: number;
  /** Set on a child compiler: the child's question. */
  subtask?: { query: string };
};

type Settings = {
  budget: number;
  claims: ClaimProvider[];
  count: TokenCounter;
  keepRecent: number;
  expectedTurns: number;
  alpha: number;
  minSavingsRatio: number;
  route: RoutePolicy | undefined;
  scorer: Scorer;
  shadowScorer: Scorer | undefined;
  applyEntry: boolean;
  minScoreTokens: number;
  subtaskQuery: string;
};

/**
 * Creates the compiler. `view` rejects only on kernel defects: a seq map
 * that disagrees with the passthrough projection, a non-finite context
 * window, or, for a subtask, a parent path without a manifest. When
 * `ctx.parentPath` is set, the call is a subtask recompile (see
 * `compileSubtask`).
 */
export function contextCompiler(opts: ContextCompilerOptions): {
  view(ctx: ViewContext): Promise<CompiledView>;
} {
  const settings: Settings = {
    budget: opts.budget,
    claims: opts.claims,
    count: opts.count ?? defaultTokenCount,
    keepRecent: opts.keepRecent ?? DEFAULT_KEEP_RECENT,
    expectedTurns: opts.expectedTurns ?? DEFAULT_EXPECTED_TURNS,
    alpha: opts.alpha ?? DEFAULT_ALPHA,
    minSavingsRatio: opts.minSavingsRatio ?? DEFAULT_MIN_SAVINGS_RATIO,
    route: opts.route,
    scorer: opts.scorer ?? ruleScorer,
    shadowScorer: opts.shadowScorer,
    applyEntry: opts.scorerApply === "entry",
    minScoreTokens: resolveMinScoreTokens(opts.minScoreTokens),
    subtaskQuery: opts.subtask?.query ?? "",
  };
  return {
    async view(ctx: ViewContext): Promise<CompiledView> {
      if (ctx.parentPath) {
        return compileSubtask(ctx, {
          budget: settings.budget,
          query: settings.subtaskQuery,
          count: settings.count,
          keepRecent: settings.keepRecent,
        });
      }
      return compileView(ctx, settings);
    },
  };
}

/**
 * One main-session compile. History is the passthrough projection,
 * including its repair and anomalies, indexed into chunks. Stages run in
 * this order: assemble the frozen and the from-scratch candidate views,
 * choose the epoch, run the rebuild scorer, open the epoch (and route),
 * place cache breakpoints, run the entry scorer, run the shadow scorer,
 * and build the manifest.
 */
async function compileView(
  ctx: ViewContext,
  settings: Settings,
): Promise<CompiledView> {
  const candidates = await assembleCandidates(ctx, settings);
  const calibration = calibrationFor(ctx.path);
  const project = projectionFor(calibration);
  const choice = chooseTurnEpoch(ctx, settings, candidates, project);
  const rebuildScored = await scoreOnRebuild(ctx, settings, candidates, choice);
  const opened = openEpoch({
    baseEpoch: candidates.epoch,
    choice: rebuildScored.choice,
    assembledStart: rebuildScored.chosen.epochStart,
    route: settings.route,
    model: ctx.model,
    budget: settings.budget,
    chunks: candidates.chunks,
    rows: rebuildScored.chosen.chunkRecords,
  });
  const cacheBreakpoints = placeBreakpoints(
    ctx,
    candidates,
    rebuildScored.chosen,
    opened.choice,
  );
  const entryScored = await scoreOnEntry(ctx, settings, {
    chosen: rebuildScored.chosen,
    chunks: candidates.chunks,
    cacheBreakpoints,
  });
  const chosen = entryScored.chosen;
  const shadow = await scoreInShadow(ctx, settings, {
    chunks: candidates.chunks,
    chosen,
    cacheBreakpoints,
  });
  return buildView(ctx, {
    chosen,
    epoch: opened.epoch,
    epochStart: opened.epochStart,
    choice: opened.choice,
    cacheBreakpoints,
    budget: budgetReport(ctx, settings, candidates, {
      chosen,
      choice: opened.choice,
      project,
    }),
    anomalies: candidates.anomalies,
    reports: {
      ...(calibration ? { calibration } : {}),
      ...(rebuildScored.report ? { scorer: rebuildScored.report } : {}),
      ...(entryScored.report ? { entryScored: entryScored.report } : {}),
      ...(shadow ? { shadow } : {}),
    },
  });
}

type ManifestBudget = {
  total: number;
  used: number;
  overrun: number;
  omitted: number;
  expands: number;
  frozenTokens: number;
  staleOmittable: number;
};

type Candidates = {
  epoch: number;
  historyCount: number;
  anomalies: CompiledView["manifest"]["anomalies"];
  seqs: number[];
  chunks: Chunk[];
  frozen: FrozenMap | undefined;
  /** The view that keeps the epoch's frozen head and chunk tiers. */
  frozenView: Assembly;
  /** A from-scratch recompile; `frozenView` itself when nothing is frozen. */
  recompiledView: Assembly;
  freeze: { frozenTokens: number; staleOmittable: number };
};

/**
 * Projects the history, indexes chunks, and assembles both candidate
 * views. Throws when the seq map disagrees with the projection.
 */
async function assembleCandidates(
  ctx: ViewContext,
  settings: Settings,
): Promise<Candidates> {
  const base = await passthroughView.view(ctx);
  const collected = await collectClaims(settings.claims, ctx);
  const seqs = projectionSeqs(ctx.path);
  assertSeqsMatch(seqs.length, base.messages.length);
  const chunks = indexChunks(base.messages, seqs, ctx.tools, settings.count);
  const frozen = frozenDecisions(ctx.path, base.epoch);
  const input = {
    path: ctx.path,
    history: base.messages,
    epoch: base.epoch,
    collected,
    chunks,
    chunkById: firstById(chunks),
    budget: settings.budget,
    keepRecent: settings.keepRecent,
    count: settings.count,
  };
  const frozenView = assembleView(input, { freezeHead: true, frozen });
  const recompiledView = frozen
    ? assembleView(input, { freezeHead: false, frozen: undefined })
    : frozenView;
  return {
    epoch: base.epoch,
    historyCount: base.messages.length,
    anomalies: base.manifest.anomalies,
    seqs,
    chunks,
    frozen,
    frozenView,
    recompiledView,
    freeze: freezeAccounting(
      frozen,
      frozenView.chunkRecords,
      recompiledView.chunkRecords,
    ),
  };
}

/**
 * `manifest.budget` for the chosen view. `omitted` counts none chunks;
 * `expands` counts assistant `expand` calls already on the path.
 */
function budgetReport(
  ctx: ViewContext,
  settings: Settings,
  candidates: Candidates,
  input: { chosen: Assembly; choice: EpochChoice; project: Projection },
): ManifestBudget {
  const { chosen } = input;
  return {
    total: settings.budget,
    used: chosen.used,
    overrun: viewOverrun({
      used: chosen.used,
      budget: settings.budget,
      project: input.project,
      contextWindow: ctx.prepared.contextWindow,
      choice: input.choice,
    }),
    omitted: chosen.chunkRecords.filter((record) => record.tier === "none")
      .length,
    expands: countExpands(ctx.path),
    frozenTokens: candidates.freeze.frozenTokens,
    staleOmittable: candidates.freeze.staleOmittable,
  };
}

/**
 * Builds the manifest and the view. Optional reports keep the key order
 * `calibration`, `scorer`, `entryScored`, `shadow` after `cache`.
 */
function buildView(
  ctx: ViewContext,
  input: {
    chosen: Assembly;
    epoch: number;
    epochStart: CompiledView["epochStart"];
    choice: EpochChoice;
    cacheBreakpoints: CompiledView["cacheBreakpoints"];
    budget: ManifestBudget;
    anomalies: CompiledView["manifest"]["anomalies"];
    reports: Partial<CompiledView["manifest"]>;
  },
): CompiledView {
  const { chosen } = input;
  const manifest = {
    epoch: input.epoch,
    turn: ctx.turn,
    budget: input.budget,
    claims: chosen.records,
    chunks: chosen.chunkRecords,
    anomalies: input.anomalies,
    epochDecision: input.choice.decision,
    cache: cacheReport(ctx),
    ...input.reports,
  };
  const view: CompiledView = {
    epoch: input.epoch,
    head: ctx.head,
    messages: chosen.messages,
    cacheBreakpoints: input.cacheBreakpoints,
    manifest,
  };
  if (input.epochStart) view.epochStart = input.epochStart;
  if (Object.keys(chosen.claimContent).length > 0) {
    view.details = { claimContent: chosen.claimContent };
  }
  return view;
}

/**
 * Picks the from-scratch view on a rebuild inside an epoch, else the
 * frozen view, then runs the rebuild scorer when this compile rebuilds in
 * `rebuild` mode. A
 * fallback appends `;scorer:fallback:<reason>` to the decision reason and
 * keeps the eliminate tiers. The score request reuses the previous turn's
 * head and messages rebuilt from the Log.
 */
async function scoreOnRebuild(
  ctx: ViewContext,
  settings: Settings,
  candidates: Candidates,
  choice: EpochChoice,
): Promise<{
  chosen: Assembly;
  choice: EpochChoice;
  report?: RebuildScorerReport;
}> {
  const chosen =
    choice.decision.rebuild && candidates.frozen
      ? candidates.recompiledView
      : candidates.frozenView;
  if (!choice.decision.rebuild || settings.applyEntry) {
    return { chosen, choice };
  }
  const scored = await runRebuildScorer({
    scorer: settings.scorer,
    chunks: candidates.chunks,
    view: scoredViewOf(chosen),
    count: settings.count,
    prevRequest: previousRequest(ctx),
    query: latestQuery(ctx.path),
  });
  return {
    chosen: withScored(chosen, scored),
    choice: scored.fallback
      ? appendReason(choice, `scorer:fallback:${scored.fallback}`)
      : choice,
    ...(scored.report ? { report: scored.report } : {}),
  };
}

/**
 * Runs the entry scorer in `entry` mode. Its request is this turn's
 * compiled view with the breakpoints already placed.
 */
async function scoreOnEntry(
  ctx: ViewContext,
  settings: Settings,
  input: {
    chosen: Assembly;
    chunks: Chunk[];
    cacheBreakpoints: CompiledView["cacheBreakpoints"];
  },
): Promise<{ chosen: Assembly; report?: EntryScoredReport }> {
  const { chosen } = input;
  if (!settings.applyEntry) return { chosen };
  const entered = await runEntryScorer({
    scorer: settings.scorer,
    chunks: input.chunks,
    view: scoredViewOf(chosen),
    count: settings.count,
    minScoreTokens: settings.minScoreTokens,
    path: ctx.path,
    prevRequest: requestFromCompiled(
      ctx,
      chosen.messages,
      input.cacheBreakpoints,
    ),
    query: latestQuery(ctx.path),
  });
  return {
    chosen: withScored(chosen, entered),
    ...(entered.report ? { report: entered.report } : {}),
  };
}

/**
 * `manifest.cache`. `expectedReadTokens` is the frozen-prefix estimate the
 * previous turn emitted (its own `head + ratio × budget.frozenTokens`), not
 * a projection with this turn's re-anchored head.
 */
function cacheReport(ctx: ViewContext): {
  expectedReadTokens: number;
  measuredReadTokens: number;
} {
  return {
    expectedReadTokens: expectedFrozenPrefix(ctx.path),
    measuredReadTokens: measuredCacheRead(ctx.path),
  };
}

function scoredViewOf(chosen: Assembly): ScoredView {
  return {
    rows: chosen.chunkRecords,
    messages: chosen.messages,
    used: chosen.used,
  };
}

function withScored(chosen: Assembly, scored: ScoredView): Assembly {
  return {
    ...chosen,
    messages: scored.messages,
    chunkRecords: scored.rows,
    used: scored.used,
  };
}

/** Runs the epoch cost check for this turn's two candidate views. */
function chooseTurnEpoch(
  ctx: ViewContext,
  settings: Settings,
  candidates: Candidates,
  project: Projection,
): EpochChoice {
  return chooseEpoch({
    frozenUsed: candidates.frozenView.used,
    recompiledUsed: candidates.recompiledView.used,
    hasFrozen: candidates.frozen !== undefined,
    project,
    rates: ratesFor(ctx.prepared.pricing),
    remainingTurns: remainingTurnsFor(
      ctx.turn,
      settings.expectedTurns,
      settings.alpha,
    ),
    minSavingsRatio: settings.minSavingsRatio,
    contextWindow: ctx.prepared.contextWindow,
    epochOpenReason: epochOpenReason(ctx.path, candidates.epoch),
  });
}

/**
 * Runs the shadow scorer in `rebuild` mode when one is configured, after
 * tiers, freeze, and budget are fixed.
 */
async function scoreInShadow(
  ctx: ViewContext,
  settings: Settings,
  input: {
    chunks: readonly Chunk[];
    chosen: Assembly;
    cacheBreakpoints: CompiledView["cacheBreakpoints"];
  },
): Promise<ShadowBody | undefined> {
  if (settings.applyEntry || !settings.shadowScorer) return undefined;
  return runShadowScorer({
    scorer: settings.shadowScorer,
    chunks: input.chunks,
    ctx,
    messages: input.chosen.messages,
    cacheBreakpoints: input.cacheBreakpoints,
    query: latestQuery(ctx.path),
  });
}

/**
 * Cache breakpoints for the chosen view. Inside an epoch the frozen prefix
 * ends at the epoch's seq ceiling; a rebuild freezes the whole history.
 */
function placeBreakpoints(
  ctx: ViewContext,
  candidates: Candidates,
  chosen: Assembly,
  choice: EpochChoice,
): CompiledView["cacheBreakpoints"] {
  return cacheBreakpointsFor({
    messageCount: chosen.messages.length,
    historyCount: candidates.historyCount,
    hasEpochHead: chosen.hasEpochHead,
    seqs: candidates.seqs,
    ceiling: choice.decision.rebuild
      ? undefined
      : frozenSeqCeiling(ctx.path, candidates.epoch),
  });
}
