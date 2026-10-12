/**
 * Log store seam.
 *
 * Guarantees: `append` is synchronous. Persistence, when there is any, finishes
 * before the entry is visible in memory. Callers must serialize `append` on
 * one sessionId. The call reads the leaf and writes the next entry; overlapping
 * appends on the same session can reuse `seq` and fork the chain by accident.
 * Different sessionIds have no such constraint.
 *
 * Intentionally not in the Log: the store object and open descriptors. The
 * next `seq` is the count of entries already accepted.
 */

import type { EntryId, EntryInput, StoredEntry } from "./types.js";

/** Append-only entry tree per session. Implemented in memory and as JSONL. */
export interface LogStore {
  /**
   * Validates, then stores. Throws `UnknownEntryKindError` or
   * `InvalidEntryError` and leaves the session unchanged on bad input.
   * `input.parentId` defaults to the current leaf.
   */
  append(sessionId: string, input: EntryInput): StoredEntry;
  /** The current leaf, or null for an empty session. */
  leaf(sessionId: string): EntryId | null;
  get(sessionId: string, id: EntryId): StoredEntry | undefined;
  /** Root to leaf. `leaf` entries themselves are not part of the chain. */
  path(sessionId: string, leafId?: EntryId): StoredEntry[];
  /** Every entry in the session, in `seq` order. */
  all(sessionId: string): readonly StoredEntry[];
}
