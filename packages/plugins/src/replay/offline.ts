/**
 * Offline policy replay. Recompiles each recorded turn from its log prefix.
 *
 * Contract: `replayOffline` does not call a model. A scorer whose `name` is
 * not `rule` is replaced with `ruleScorer`, and the returned array carries a
 * non-enumerable `scorerNote` of `scorer:offline-rule` that `compareOffline`
 * copies into the report. `recordedTokens` is that turn's assistant
 * `details.usage` input total (`inputOther + inputCacheRead +
 * inputCacheCreation`). Passthrough sets `policyTokens` to that total, so
 * `saved` is 0. A compiler policy sets `policyTokens` to `budget.used`
 * times `manifest.calibration.ratio`, or 1 when that turn has no
 * calibration. The log does not store `contextWindow` or pricing; replay
 * uses 8000 and null, the same stand-in as keyless replay.
 * Each turn's path is the recorded prefix before that turn's view commit
 * (`model_change`, `view_epoch`, `request_header`, and everything after).
 * Compiler policies then keep their own prior manifests on a shadow log so
 * later turns freeze the policy under test, not the recorded one.
 * `prefixStable` is true on the first turn, and afterwards when the previous
 * recompiled head matches and its messages are a JSON prefix of this turn.
 * `epoch`, `rebuild`, `oldTokens`, `newTokens`, `decisionReason`, and
 * `chunkTiers` copy that turn's recompiled manifest. `compareOffline` does
 * not print them.
 * `omittedIds` are manifest chunks at tier `none`. `likelyNeededOmissions`
 * are omitted tool chunks whose `toolUseId` (the id after `#`) or an
 * `accesses.key` occurs in a later assistant's text or tool_use input.
 * `compareOffline` reports per-turn policy tokens, total saved, prefix-stable
 * turn counts, and likely-needed counts. The report is numbers, ids, and
 * the scorer note only.
 *
 * Intentionally not in the Log: the shadow log, the diffs, and the report.
 * The recorded session is the fact. Replay does not append to it.
 */

import type { CompiledView, Manifest, ViewContext } from "@sage/core";
import {
  type HarnessTool,
  type LogStore,
  MemoryLogStore,
  passthroughView,
  type StoredEntry,
  systemHash,
  toolsHash,
} from "@sage/core";
import type { Message, PreparedCall } from "@sage/provider/contract";
import { contextCompiler } from "../compiler/compiler.js";
import { ruleScorer } from "../scorer/rule.js";
import type { Scorer } from "../scorer/types.js";

export type Policy = {
  budget?: number;
  keepRecent?: number;
  scorer?: Scorer;
  expectedTurns?: number;
  passthrough?: boolean;
};

export type TurnDiff = {
  turn: number;
  recordedTokens: number;
  policyTokens: number;
  saved: number;
  prefixStable: boolean;
  omittedIds: string[];
  likelyNeededOmissions: string[];
  epoch: number;
  rebuild: boolean;
  oldTokens: number;
  newTokens: number;
  decisionReason: string;
  chunkTiers: { id: string; tier: string }[];
};

const SCORER_NOTE = "scorer:offline-rule";
const REPLAY_WINDOW = 8000;

type Slice = {
  turn: number;
  prefixEnd: number;
  assistant: StoredEntry;
  header: StoredEntry;
};

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
  const swapped = policy.scorer !== undefined && policy.scorer.name !== "rule";
  const scorer = swapped ? ruleScorer : policy.scorer;
  const provider = policy.passthrough
    ? passthroughView
    : contextCompiler({
        budget: policy.budget ?? 40_000,
        claims: [],
        keepRecent: policy.keepRecent ?? 6,
        expectedTurns: policy.expectedTurns ?? 20,
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

  if (swapped) {
    Object.defineProperty(diffs, "scorerNote", {
      value: SCORER_NOTE,
      enumerable: false,
    });
  }
  return diffs;
}

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
    const note = (diffs as { scorerNote?: unknown }).scorerNote;
    if (note === SCORER_NOTE && !notes.includes(note)) notes.push(note);
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
  let latest = 0;
  let seen = false;
  for (const entry of path) {
    if (entry.kind !== "view_epoch" || !isRecord(entry.payload)) continue;
    const epoch = finite(entry.payload.epoch);
    if (epoch === undefined) continue;
    latest = epoch;
    seen = true;
  }
  const baseline = seen ? latest : 0;
  if (view.epochStart || view.epoch > baseline) {
    const epochHead = view.messages[0];
    shadow.append(sessionId, {
      kind: "view_epoch",
      payload: {
        epoch: view.epoch,
        reason: view.epochStart?.reason ?? "epoch-advanced",
      },
      ...(epochHead ? { details: { epochHead } } : {}),
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

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function finite(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : undefined;
}
