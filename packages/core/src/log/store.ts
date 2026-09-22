/**
 * Log store seam.
 *
 * Contract: `append` is synchronous. Persistence, when there is any, finishes
 * before the entry is visible in memory. Callers must serialize `append` on
 * one sessionId. The call reads the leaf and writes the next entry; overlapping
 * appends on the same session can reuse `seq` and fork the chain by accident.
 * Different sessionIds have no such constraint.
 *
 * `replay` folds entries with the pure reducers registered for their kinds.
 * Kinds without a reducer are skipped. The reducer does not receive the store.
 *
 * Intentionally not in the Log: the store object, open descriptors, and any
 * high-water mark. The next `seq` is the count of entries already accepted.
 */

import { kindDef } from "./registry.js";
import type { EntryId, EntryInput, StoredEntry } from "./types.js";

export interface LogStore {
  append(sessionId: string, input: EntryInput): StoredEntry;
  leaf(sessionId: string): EntryId | null;
  get(sessionId: string, id: EntryId): StoredEntry | undefined;
  /** Root to leaf. `leaf` entries themselves are not part of the chain. */
  path(sessionId: string, leafId?: EntryId): StoredEntry[];
  /** Every entry in the session, in `seq` order. */
  all(sessionId: string): readonly StoredEntry[];
}

export function replay<S>(entries: readonly StoredEntry[], init: S): S {
  const ordered = [...entries].sort((a, b) => a.seq - b.seq);
  let state = init;
  for (const entry of ordered) {
    const reduce = kindDef(entry.kind)?.reduce;
    if (!reduce) continue;
    state = reduce(state, entry) as S;
  }
  return state;
}
