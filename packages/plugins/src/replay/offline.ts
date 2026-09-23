/**
 * Offline policy replay: recompiles each recorded turn under another
 * policy and compares token estimates, without calling any model.
 *
 * Guarantees: each turn's view is compiled from the recorded prefix before
 * that turn's view commit (`model_change`, `view_epoch`, `request_header`,
 * and everything after). Compiler policies keep their own prior manifests
 * on a shadow log, so later turns freeze the policy under test, not the
 * recorded one. A scorer that would call out is swapped for `ruleScorer`,
 * and the report says so. The report holds numbers, ids, and the scorer
 * note only, never chunk text. Per-field meanings: `TurnDiff`.
 *
 * Intentionally not in the Log: the shadow log, the diffs, and the report.
 * The recorded session is the fact. Replay does not append to it.
 */

import type { CompiledView, Manifest, ViewContext } from "@sage/core";
import {
  type HarnessTool,
  isRecord,
  type LogStore,
  latestViewEpoch,
  MemoryLogStore,
  passthroughView,
  type StoredEntry,
  systemHash,
  toolsHash,
} from "@sage/core";
import type { Message, PreparedCall } from "@sage/provider/contract";
import { contextCompiler, DEFAULT_KEEP_RECENT } from "../compiler/compiler.js";
import { DEFAULT_EXPECTED_TURNS } from "../compiler/epoch.js";
import { ruleScorer } from "../scorer/rule.js";
import type { Scorer } from "../scorer/types.js";

/** The policy a replay recompiles under. */
export type Policy = {
  /** Defaults to `REPLAY_DEFAULT_BUDGET`. */
  budget?: number;
  /** Defaults to `DEFAULT_KEEP_RECENT`. */
  keepRecent?: number;
  /** Without `isNoop`, replaced by `ruleScorer`. */
  scorer?: Scorer;
  /** Defaults to `DEFAULT_EXPECTED_TURNS`. */
  expectedTurns?: number;
  /** Replay the passthrough view instead of the compiler. */
  passthrough?: boolean;
};

/** One replayed turn: the recorded cost next to the policy's estimate. */
export type TurnDiff = {
  turn: number;
  /**
   * That turn's assistant `details.usage` input total
   * (`inputOther + inputCacheRead + inputCacheCreation`).
   */
  recordedTokens: number;
  /**
   * Passthrough: `recordedTokens`, so `saved` is 0. Compiler policy:
   * `budget.used` times `manifest.calibration.ratio`, or times 1 when that
   * turn has no calibration.
   */
  policyTokens: number;
  /** `recordedTokens - policyTokens`. */
  saved: number;
  /**
   * True on the first turn; afterwards true when the previous recompiled
   * head matches and its messages are a JSON prefix of this turn's.
   */
  prefixStable: boolean;
  /** Manifest chunk ids at tier `none`. */
  omittedIds: string[];
  /**
   * Omitted tool chunks whose `toolUseId` (the id after `#`) or an
   * `accesses(input)` key occurs in a later assistant's text or tool_use
   * input: a proxy for "referenced later".
   */
  likelyNeededOmissions: string[];
  /**
   * This and the fields below copy the recompiled manifest;
   * `compareOffline` does not print them.
   */
  epoch: number;
  rebuild: boolean;
  oldTokens: number;
  newTokens: number;
  decisionReason: string;
  chunkTiers: { id: string; tier: string }[];
};

/** Default replay budget, the same as the sage CLI's default budget. */
const REPLAY_DEFAULT_BUDGET = 40_000;
const SCORER_NOTE = "scorer:offline-rule";
/** Results of `replayOffline` runs whose scorer was swapped for `ruleScorer`. */
const scorerSwappedRuns = new WeakSet<readonly TurnDiff[]>();
/**
 * The Log stores neither the context window nor pricing; replay uses this
 * window and `pricing: null`, the same stand-in as keyless replay.
 */
const REPLAY_WINDOW = 8000;

type Slice = {
  turn: number;
  prefixEnd: number;
  assistant: StoredEntry;
  header: StoredEntry;
};

/**
 * Replays every recorded turn of `sessionId` under `policy` and returns one
 * diff per turn that has an assistant reply. Reads `store` only. When the
 * scorer was swapped, `compareOffline` notes it for this returned array
 * (the array itself is remembered, not a copy). Throws when a recorded
 * `request_header` is not an object or lacks provider or model.
 */
export async function replayOffline(
  store: LogStore,
  sessionId: string,
  policy: Policy,
  deps: { tools: HarnessTool[]; system: string },
): Promise<TurnDiff[]> {
  const recorded = store.path(sessionId);
  const slices = sliceTurns(recorded);
  const shadow = new MemoryLogStore();
  const shadowId = "offline";
  let copiedThrough = 0;
  const views: CompiledView[] = [];
  const diffs: TurnDiff[] = [];
  const swapped = policy.scorer !== undefined && !policy.scorer.isNoop;
  const scorer = swapped ? ruleScorer : policy.scorer;
  const provider = policy.passthrough
    ? passthroughView
    : contextCompiler({
        budget: policy.budget ?? REPLAY_DEFAULT_BUDGET,
        claims: [],
        keepRecent: policy.keepRecent ?? DEFAULT_KEEP_RECENT,
        expectedTurns: policy.expectedTurns ?? DEFAULT_EXPECTED_TURNS,
        ...(scorer ? { scorer } : {}),
      });

  for (const slice of slices) {
    if (!policy.passthrough) {
      copyMessages(recorded, copiedThrough, slice.prefixEnd, shadow, shadowId);
    }
    copiedThrough = slice.prefixEnd;
    const prefix = policy.passthrough
      ? recorded.slice(0, slice.prefixEnd)
      : shadow.path(shadowId);
    const ctx = viewContext(slice, prefix, deps);
    const view = await provider.view(ctx);
    const previous = views.at(-1);
    views.push(view);
    const recordedTokens = inputTotal(slice.assistant);
    const policyTokens = policy.passthrough
      ? recordedTokens
      : estimateTokens(view.manifest);
    const omittedIds = omittedChunkIds(view.manifest);
    const decision = view.manifest.epochDecision;
    diffs.push({
      turn: slice.turn,
      recordedTokens,
      policyTokens,
      saved: recordedTokens - policyTokens,
      prefixStable: previous ? prefixStable(previous, view) : true,
      omittedIds,
      likelyNeededOmissions: likelyNeeded(
        omittedIds,
        recorded,
        slice,
        deps.tools,
      ),
      epoch: view.epoch,
      rebuild: decision?.rebuild ?? false,
      oldTokens: decision?.oldTokens ?? 0,
      newTokens: decision?.newTokens ?? 0,
      decisionReason: decision?.reason ?? "",
      chunkTiers: view.manifest.chunks.map((chunk) => ({
        id: chunk.id,
        tier: chunk.tier,
      })),
    });
    if (!policy.passthrough) commitView(shadow, shadowId, view, ctx);
  }

  if (swapped) scorerSwappedRuns.add(diffs);
  return diffs;
}

/**
 * Compares two replays: per-turn `policyTokens`, total `saved`,
 * prefix-stable turn counts, and likely-needed omission counts, as a
 * Markdown table and the same numbers as JSON. Adds `scorer:offline-rule`
 * once when either replay swapped its scorer.
 */
export function compareOffline(
  a: TurnDiff[],
  b: TurnDiff[],
): { markdown: string; json: unknown } {
  const turns = [
    ...new Set([...a.map((diff) => diff.turn), ...b.map((diff) => diff.turn)]),
  ].sort((left, right) => left - right);
  const json = {
    turns: turns.map((turn) => ({
      turn,
      a: policyTokensAt(a, turn),
      b: policyTokensAt(b, turn),
    })),
    saved: { a: sumSaved(a), b: sumSaved(b) },
    prefixStableTurns: {
      a: a.filter((diff) => diff.prefixStable).length,
      b: b.filter((diff) => diff.prefixStable).length,
    },
    likelyNeededOmissions: {
      a: countLikely(a),
      b: countLikely(b),
    },
    notes: notesOf(a, b),
  };
  const lines = [
    "| turn | A | B |",
    "|---:|---:|---:|",
    ...json.turns.map(
      (row) => `| ${row.turn} | ${formatCell(row.a)} | ${formatCell(row.b)} |`,
    ),
    "",
    `saved A ${json.saved.a} B ${json.saved.b}`,
    `prefixStableTurns A ${json.prefixStableTurns.a} B ${json.prefixStableTurns.b}`,
    `likelyNeededOmissions A ${json.likelyNeededOmissions.a} B ${json.likelyNeededOmissions.b}`,
    ...json.notes,
  ];
  return { markdown: `${lines.join("\n")}\n`, json };
}

function policyTokensAt(
  diffs: readonly TurnDiff[],
  turn: number,
): number | null {
  const found = diffs.find((diff) => diff.turn === turn);
  return found ? found.policyTokens : null;
}

function sumSaved(diffs: readonly TurnDiff[]): number {
  return diffs.reduce((total, diff) => total + diff.saved, 0);
}

function countLikely(diffs: readonly TurnDiff[]): number {
  return diffs.reduce(
    (total, diff) => total + diff.likelyNeededOmissions.length,
    0,
  );
}

function notesOf(a: readonly TurnDiff[], b: readonly TurnDiff[]): string[] {
  const notes: string[] = [];
  for (const diffs of [a, b]) {
    if (scorerSwappedRuns.has(diffs) && !notes.includes(SCORER_NOTE)) {
      notes.push(SCORER_NOTE);
    }
  }
  return notes;
}

function formatCell(value: number | null): string {
  return value === null ? "" : String(value);
}

function sliceTurns(path: readonly StoredEntry[]): Slice[] {
  const slices: Slice[] = [];
  for (let index = 0; index < path.length; index += 1) {
    const entry = path[index];
    if (entry?.kind !== "request_header") continue;
    let prefixEnd = index;
    while (prefixEnd > 0) {
      const prev = path[prefixEnd - 1];
      if (prev?.kind !== "view_epoch" && prev?.kind !== "model_change") break;
      prefixEnd -= 1;
    }
    let turn = slices.length + 1;
    let assistant: StoredEntry | undefined;
    for (let cursor = index + 1; cursor < path.length; cursor += 1) {
      const next = path[cursor];
      if (!next || next.kind === "request_header") break;
      if (next.kind === "manifest" && isRecord(next.payload)) {
        const value = finite(next.payload.turn);
        if (value !== undefined) turn = value;
      }
      if (isAssistantEntry(next)) {
        assistant = next;
        break;
      }
    }
    if (!assistant) continue;
    slices.push({ turn, prefixEnd, assistant, header: entry });
  }
  return slices;
}

function copyMessages(
  recorded: readonly StoredEntry[],
  from: number,
  to: number,
  shadow: MemoryLogStore,
  sessionId: string,
): void {
  for (let index = from; index < to; index += 1) {
    const entry = recorded[index];
    if (entry?.kind !== "message") continue;
    shadow.append(sessionId, {
      kind: "message",
      payload: entry.payload,
      ...(entry.details ? { details: entry.details } : {}),
    });
  }
}

function commitView(
  shadow: MemoryLogStore,
  sessionId: string,
  view: CompiledView,
  ctx: ViewContext,
): void {
  const path = shadow.path(sessionId);
  if (view.epochStart || view.epoch > latestViewEpoch(path)) {
    const prefixMessage = view.messages[0];
    shadow.append(sessionId, {
      kind: "view_epoch",
      payload: {
        epoch: view.epoch,
        reason: view.epochStart?.reason ?? "epoch-advanced",
      },
      ...(prefixMessage ? { details: { prefixMessage } } : {}),
    });
  }
  shadow.append(sessionId, {
    kind: "request_header",
    payload: {
      provider: ctx.model.provider,
      model: ctx.model.model,
      systemPromptHash: systemHash(ctx.head.system),
      toolsHash: toolsHash(ctx.head.tools),
      sampling: {},
      adapterDefaults: { ...ctx.prepared.adapterDefaults },
      system: ctx.head.system,
      tools: ctx.head.tools,
    },
  });
  shadow.append(sessionId, {
    kind: "manifest",
    payload: view.manifest,
    ...(view.details ? { details: view.details } : {}),
  });
}

function viewContext(
  slice: Slice,
  prefix: readonly StoredEntry[],
  deps: { tools: HarnessTool[]; system: string },
): ViewContext {
  const header = headerOf(slice.header);
  const model = { provider: header.provider, model: header.model };
  const prepared: PreparedCall = {
    model,
    contextWindow: REPLAY_WINDOW,
    pricing: null,
    adapterDefaults: header.adapterDefaults,
  };
  return {
    sessionId: "offline",
    turn: slice.turn,
    path: prefix,
    model,
    prepared,
    head: {
      system: deps.system,
      tools: deps.tools.map((tool) => tool.def),
    },
    tools: deps.tools,
    depth: 0,
  };
}

function headerOf(entry: StoredEntry): {
  provider: string;
  model: string;
  adapterDefaults: Record<string, unknown>;
} {
  if (!isRecord(entry.payload)) {
    throw new Error("request_header payload is not an object");
  }
  const provider = entry.payload.provider;
  const model = entry.payload.model;
  if (typeof provider !== "string" || typeof model !== "string") {
    throw new Error("request_header is missing provider or model");
  }
  const defaults = entry.payload.adapterDefaults;
  return {
    provider,
    model,
    adapterDefaults: isRecord(defaults) ? defaults : {},
  };
}

function estimateTokens(manifest: Manifest): number {
  const used = budgetUsed(manifest.budget);
  const ratio = manifest.calibration?.ratio;
  const scale = typeof ratio === "number" && Number.isFinite(ratio) ? ratio : 1;
  return used * scale;
}

function budgetUsed(budget: Manifest["budget"]): number {
  if (typeof budget === "number" && Number.isFinite(budget)) return budget;
  if (!budget || typeof budget !== "object") return 0;
  return finite(budget.used) ?? 0;
}

function omittedChunkIds(manifest: Manifest): string[] {
  const ids: string[] = [];
  for (const chunk of manifest.chunks) {
    if (chunk.tier !== "none") continue;
    ids.push(chunk.id);
  }
  return ids;
}

function likelyNeeded(
  omittedIds: readonly string[],
  recorded: readonly StoredEntry[],
  slice: Slice,
  tools: readonly HarnessTool[],
): string[] {
  if (omittedIds.length === 0) return [];
  const later = laterAssistantBlob(recorded, slice.assistant);
  if (later.length === 0) return [];
  const hits: string[] = [];
  for (const id of omittedIds) {
    const toolUseId = toolUseIdOf(id);
    if (!toolUseId) continue;
    const call = findToolUse(recorded, slice.prefixEnd, toolUseId);
    if (!call) continue;
    const tool = tools.find((item) => item.def.name === call.name);
    const keys = tool
      ? tool.accesses(call.input).map((access) => access.key)
      : [];
    const needles = [toolUseId, ...keys].filter((needle) => needle.length > 0);
    if (needles.some((needle) => later.includes(needle))) hits.push(id);
  }
  return hits;
}

function laterAssistantBlob(
  recorded: readonly StoredEntry[],
  assistant: StoredEntry,
): string {
  const start = recorded.indexOf(assistant);
  if (start < 0) return "";
  const parts: string[] = [];
  for (let index = start + 1; index < recorded.length; index += 1) {
    const entry = recorded[index];
    if (!entry || !isAssistantEntry(entry)) continue;
    parts.push(assistantBlob(entry));
  }
  return parts.join("\n");
}

function assistantBlob(entry: StoredEntry): string {
  if (!isMessage(entry.payload)) return "";
  const parts: string[] = [];
  for (const block of entry.payload.content) {
    if (block.type === "text" || block.type === "thinking") {
      parts.push(block.text);
      continue;
    }
    if (block.type !== "tool_use") continue;
    parts.push(block.rawInput);
    parts.push(JSON.stringify(block.input) ?? "");
  }
  return parts.join("\n");
}

function findToolUse(
  recorded: readonly StoredEntry[],
  prefixEnd: number,
  toolUseId: string,
): { name: string; input: unknown } | undefined {
  for (let index = 0; index < prefixEnd; index += 1) {
    const entry = recorded[index];
    if (entry?.kind !== "message" || !isMessage(entry.payload)) {
      continue;
    }
    for (const block of entry.payload.content) {
      if (block.type === "tool_use" && block.id === toolUseId) {
        return { name: block.name, input: block.input };
      }
    }
  }
  return undefined;
}

function toolUseIdOf(chunkId: string): string | undefined {
  const mark = chunkId.indexOf("#");
  if (mark < 0) return undefined;
  const id = chunkId.slice(mark + 1);
  return id.length > 0 ? id : undefined;
}

function prefixStable(previous: CompiledView, next: CompiledView): boolean {
  if (previous.head.system !== next.head.system) return false;
  if (JSON.stringify(previous.head.tools) !== JSON.stringify(next.head.tools)) {
    return false;
  }
  if (previous.messages.length > next.messages.length) return false;
  return previous.messages.every(
    (message, index) =>
      JSON.stringify(message) === JSON.stringify(next.messages[index]),
  );
}

function inputTotal(entry: StoredEntry): number {
  const usage = entry.details?.usage;
  if (!isRecord(usage)) return 0;
  return (
    (finite(usage.inputOther) ?? 0) +
    (finite(usage.inputCacheRead) ?? 0) +
    (finite(usage.inputCacheCreation) ?? 0)
  );
}

function isAssistantEntry(entry: StoredEntry): boolean {
  return entry.kind === "message" && isAssistant(entry.payload);
}

function isAssistant(payload: unknown): boolean {
  return isRecord(payload) && payload.role === "assistant";
}

function isMessage(payload: unknown): payload is Message {
  if (!isRecord(payload)) return false;
  return (
    (payload.role === "user" || payload.role === "assistant") &&
    Array.isArray(payload.content)
  );
}

function finite(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : undefined;
}
