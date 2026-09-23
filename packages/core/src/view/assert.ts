/**
 * Seam assertions and content-addressed head hashes.
 *
 * Contract: tool_use and tool_result ids must already be paired. The kernel
 * does not repair them. A head hash may differ from the previous request
 * header only when `view.epoch` is greater than `currentEpoch`. `currentEpoch`
 * is the epoch of that previous header (latest view_epoch at or before it),
 * not the newest epoch on the path.
 *
 * Hashes are sha256 of canonical JSON (object keys sorted, array order kept).
 *
 * Intentionally not in the Log: hash functions and thrown errors.
 */

import { createHash } from "node:crypto";
import {
  type Head,
  isRecord,
  type Message,
  type ToolDef,
} from "@sage/provider/contract";
import type { RequestHeaderPayload } from "../log/kinds.js";
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

export function systemHash(system: string): string {
  return sha256(canonicalJson(system));
}

export function toolsHash(tools: readonly ToolDef[]): string {
  return sha256(canonicalJson(tools));
}

export function headHash(head: Head): string {
  return sha256(canonicalJson(head));
}

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

export function assertHeadFrozen(
  view: CompiledView,
  prevHeader: RequestHeaderPayload | undefined,
  currentEpoch: number,
): void {
  if (!prevHeader) return;
  const sameSystem =
    prevHeader.systemPromptHash === systemHash(view.head.system);
  const sameTools = prevHeader.toolsHash === toolsHash(view.head.tools);
  if (sameSystem && sameTools) return;
  if (view.epoch > currentEpoch) return;
  throw new HeadChangedMidEpochError();
}
