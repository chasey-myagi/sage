/**
 * Kernel public surface.
 *
 * Contract: importing this module registers the built-in log kinds. Callers
 * see Log and, later, the View seam. This package does not import plugins.
 *
 * Intentionally not in the Log: package identity and the kind registry.
 */

import "./log/kinds.js";

export const PACKAGE = "@sage/core" as const;

export {
  InvalidEntryError,
  LogCorruptedError,
  UnknownEntryKindError,
} from "./log/errors.js";
export { JsonlLogStore, type LogFs } from "./log/jsonl.js";
export { MemoryLogStore } from "./log/memory.js";
export {
  type EntryKindDef,
  entryKinds,
  registerEntryKind,
} from "./log/registry.js";
export {
  DANGLING,
  synthesizeToolResult,
  type ToolResultBlock,
} from "./log/repair.js";
export { type LogStore, replay } from "./log/store.js";
export type {
  EntryId,
  EntryInput,
  MessagePayload,
  ModelRef,
  StoredEntry,
} from "./log/types.js";
