/**
 * Seam assertions and content-addressed head hashes. The head is the
 * request's system prompt plus tool schemas.
 *
 * Guarantees: an unpaired view or a head changed inside an epoch throws
 * before anything is sent; nothing here repairs. Hashes are sha256 of
 * canonical JSON (object keys sorted, undefined keys dropped, array order
 * kept), so equal values hash equal regardless of key order.
 *
 * Intentionally not in the Log: hash functions and thrown errors.
 */

import { createHash } from "node:crypto";
import { isRecord, type Message, type ToolDef } from "@sage/provider/contract";
import type { RequestHeaderPayload } from "../log/kinds.js";
import {
  lastHeaderIndex,
  lastRequestHeader,
  latestViewEpoch,
} from "../log/path.js";
import type { StoredEntry } from "../log/types.js";
import { HeadChangedMidEpochError, UnpairedToolCallError } from "./errors.js";
import type { CompiledView } from "./types.js";

function canonicalJson(value: unknown): string {
  if (value === undefined) return "null";
  if (Array.isArray(value)) {
    return `[${value
      .map((item) => canonicalJson(item === undefined ? null : item))
      .join(",")}]`;
  }
  if (!isRecord(value)) return JSON.stringify(value) ?? "null";
  const keys = Object.keys(value)
    .filter((key) => value[key] !== undefined)
    .sort();
  return `{${keys
    .map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`)
    .join(",")}}`;
}

function sha256(text: string): string {
  return createHash("sha256").update(text).digest("hex");
}

/** Hex sha256 of the system prompt, as stored in `systemPromptHash`. */
export function systemHash(system: string): string {
  return sha256(canonicalJson(system));
}

/** Hex sha256 of the tool schemas, as stored in `request_header.toolsHash`. */
export function toolsHash(tools: readonly ToolDef[]): string {
  return sha256(canonicalJson(tools));
}

/**
 * Throws `UnpairedToolCallError` unless every tool_use id appears once and
 * is answered by exactly one later tool_result, and every tool_result
 * answers an earlier open tool_use.
 */
export function assertPaired(messages: Message[]): void {
  const open = new Set<string>();
  for (const message of messages) {
    for (const block of message.content) {
      if (block.type === "tool_use") {
        if (open.has(block.id)) throw new UnpairedToolCallError(block.id);
        open.add(block.id);
        continue;
      }
      if (block.type === "tool_result" && !open.delete(block.toolUseId)) {
        throw new UnpairedToolCallError(block.toolUseId);
      }
    }
  }
  const first = open.values().next().value;
  if (first !== undefined) throw new UnpairedToolCallError(first);
}

/** True when `view`'s system and tools hash to what `header` recorded. */
function headMatches(
  header: RequestHeaderPayload,
  view: CompiledView,
): boolean {
  return (
    header.systemPromptHash === systemHash(view.head.system) &&
    header.toolsHash === toolsHash(view.head.tools)
  );
}

/**
 * Throws `HeadChangedMidEpochError` when the head hash differs from
 * `prevHeader` and `view.epoch` is not greater than `currentEpoch`, the
 * epoch the previous request was sent in (the latest view_epoch at or
 * before that header, not the newest on the path). No previous header
 * means no constraint.
 *
 * This checks only that the epoch advanced since the last request; it is
 * part two of `assertHeadFrozenOnPath`, which the session calls.
 */
export function assertHeadFrozen(
  view: CompiledView,
  prevHeader: RequestHeaderPayload | undefined,
  currentEpoch: number,
): void {
  if (!prevHeader || headMatches(prevHeader, view)) return;
  if (view.epoch > currentEpoch) return;
  throw new HeadChangedMidEpochError();
}

/**
 * The session's head-freeze check against its Log path. Throws
 * `HeadChangedMidEpochError` unless the head hash matches the last
 * request_header, or `view.epoch === latest > frozen`:
 *
 * - Part one, `view.epoch === latest`: the view's epoch must be the newest
 *   view_epoch already on the path. A view cannot change the head by
 *   claiming an epoch the Log does not hold yet, so its view_epoch must be
 *   on the path when `view()` returns. The session appends `epochStart`'s
 *   view_epoch only after this check, so a head change in the turn that
 *   opens the epoch through `epochStart` is rejected.
 * - Part two, `view.epoch > frozen`: `assertHeadFrozen` with `frozen`, the
 *   epoch the last request was sent in (the newest view_epoch at or before
 *   the last request_header).
 *
 * The two are different conditions, not one check done twice: part one
 * alone passes a head change inside the epoch the last request already
 * used; part two alone passes an epoch number no view_epoch records.
 */
export function assertHeadFrozenOnPath(
  view: CompiledView,
  path: readonly StoredEntry[],
): void {
  const prev = lastRequestHeader(path);
  const header = lastHeaderIndex(path);
  const latest = latestViewEpoch(path);
  const frozen =
    header < 0 ? latest : latestViewEpoch(path.slice(0, header + 1));
  if (prev && !headMatches(prev, view) && view.epoch !== latest) {
    throw new HeadChangedMidEpochError();
  }
  assertHeadFrozen(view, prev, frozen);
}
