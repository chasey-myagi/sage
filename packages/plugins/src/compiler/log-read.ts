/**
 * Read-only queries over a session path for the compiler.
 *
 * Contract: every function here reads entries the kernel already wrote and
 * returns fresh values; nothing appends, and nothing mutates the path.
 * Payloads are untrusted JSON, so each reader validates before it uses a
 * field and treats a malformed entry as absent.
 *
 * Intentionally not in the Log: these readers and their results. They only
 * rebuild what `message`, `manifest`, `view_epoch`, and `request_header`
 * entries already record.
 */

import { isRecord, type ViewContext } from "@sage/core";
import type { Message, Request, Sampling } from "@sage/provider/contract";
import { EXPAND_TOOL_NAME } from "./expand.js";
import { rebuildFromLog } from "./rebuild.js";

/** A session path as the view provider receives it. */
export type SessionPath = ViewContext["path"];
/** One entry of a `SessionPath`. */
export type PathEntry = SessionPath[number];

/** Narrows an untrusted value to a user or assistant `Message`. */
export function isMessage(value: unknown): value is Message {
  if (!isRecord(value)) return false;
  return (
    (value.role === "user" || value.role === "assistant") &&
    Array.isArray(value.content)
  );
}

/** Returns the last `manifest` entry on the path. */
export function latestManifest(path: SessionPath): PathEntry | undefined {
  for (let index = path.length - 1; index >= 0; index -= 1) {
    const entry = path[index];
    if (entry?.kind === "manifest") return entry;
  }
  return undefined;
}

/** Returns a manifest entry's `turn`, or undefined when it is not a number. */
export function manifestTurn(entry: PathEntry | undefined): number | undefined {
  if (!entry || !isRecord(entry.payload)) return undefined;
  return typeof entry.payload.turn === "number"
    ? entry.payload.turn
    : undefined;
}

/**
 * Returns the chunk ids listed on the latest manifest. A chunk absent from
 * this set is new since the previous turn.
 */
export function listedChunkIds(path: SessionPath): Set<string> {
  const ids = new Set<string>();
  const manifest = latestManifest(path);
  if (
    !manifest ||
    !isRecord(manifest.payload) ||
    !Array.isArray(manifest.payload.chunks)
  ) {
    return ids;
  }
  for (const chunk of manifest.payload.chunks) {
    if (!isRecord(chunk) || typeof chunk.id !== "string") continue;
    ids.add(chunk.id);
  }
  return ids;
}

/**
 * Returns the `contentHash` a manifest recorded for claim `id`, or undefined
 * when that manifest did not list the claim with a hash.
 */
export function previousHash(
  manifest: PathEntry | undefined,
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

/**
 * Returns a clone of the last `view_epoch.details.prefixMessage` recorded for
 * `epoch`, or undefined when that epoch stored none.
 */
export function prefixMessageFor(
  path: SessionPath,
  epoch: number,
): Message | undefined {
  let found: Message | undefined;
  for (const entry of path) {
    if (entry.kind !== "view_epoch") continue;
    if (!isRecord(entry.payload) || entry.payload.epoch !== epoch) continue;
    const stored = entry.details?.prefixMessage;
    if (!isMessage(stored)) continue;
    found = structuredClone(stored);
  }
  return found;
}

/**
 * True when `message` is byte-identical (as JSON) to a logged `message`
 * entry. Such an epoch head is history, not a rendered claim message.
 */
export function isLoggedMessage(path: SessionPath, message: Message): boolean {
  const rendered = JSON.stringify(message);
  for (const entry of path) {
    if (entry.kind !== "message") continue;
    if (JSON.stringify(entry.payload) === rendered) return true;
  }
  return false;
}

/**
 * Maps each passthrough-projected message to its Log `seq`. A synthetic
 * message the projection inserts to close dangling tool_use blocks gets -1.
 * Callers check that the length matches the projected message count.
 */
export function projectionSeqs(path: SessionPath): number[] {
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

/**
 * Throws when the seq map and the passthrough projection disagree in
 * length. That is a kernel defect, not a recoverable input.
 */
export function assertSeqsMatch(seqCount: number, messageCount: number): void {
  if (seqCount !== messageCount) {
    throw new Error(
      `projected seqs (${seqCount}) do not match passthrough messages (${messageCount})`,
    );
  }
}

/**
 * Maps a chunk id to one `expanded_at:<turn>` stamp per assistant `expand`
 * tool_use that names it. `<turn>` is the turn of the manifest preceding
 * that call, or 0 before the first manifest.
 */
export function expandStamps(path: SessionPath): Map<string, string[]> {
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
      if (block.type !== "tool_use" || block.name !== EXPAND_TOOL_NAME)
        continue;
      const id = expandTarget(block.input);
      if (!id) continue;
      const list = stamps.get(id) ?? [];
      list.push(`expanded_at:${turn}`);
      stamps.set(id, list);
    }
  }
  return stamps;
}

/** Counts assistant `expand` tool_use blocks on the path, valid or not. */
export function countExpands(path: SessionPath): number {
  let total = 0;
  for (const entry of path) {
    if (entry.kind !== "message" || !isMessage(entry.payload)) continue;
    if (entry.payload.role !== "assistant") continue;
    for (const block of entry.payload.content) {
      if (block.type === "tool_use" && block.name === EXPAND_TOOL_NAME)
        total += 1;
    }
  }
  return total;
}

function expandTarget(input: unknown): string | undefined {
  if (!isRecord(input)) return undefined;
  if (typeof input.id !== "string" || input.id.length === 0) return undefined;
  return input.id;
}

/**
 * Returns the summary notes the first manifest of `epoch` stored on its
 * tier-summary chunks, keyed by chunk id.
 */
export function loggedSummaries(
  path: SessionPath,
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
  path: SessionPath,
  epoch: number,
): PathEntry | undefined {
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

/**
 * Returns the text of the latest user message that has any text blocks,
 * joined with newlines, or "" when there is none.
 */
export function latestQuery(path: SessionPath): string {
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

/**
 * Returns provider, model, and sampling from the latest `request_header`
 * that names both a provider and a model.
 */
export function latestHeader(
  path: SessionPath,
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

/**
 * Rebuilds the previous turn's request from the Log: head and messages via
 * `rebuildFromLog`, breakpoints at head and the last message. Returns
 * undefined when there is no manifest or header, or the rebuild throws.
 */
export function previousRequest(ctx: ViewContext): Request | undefined {
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
    // A path the rebuild cannot replay must not fail the compile; the scorer
    // receives no previous request and decides its own fallback.
    return undefined;
  }
}
