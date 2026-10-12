/**
 * Log entry types.
 *
 * Guarantees: a stored entry is pure JSON (`id`, `parentId`, `seq`, `at`,
 * `kind`, `payload`, optional `details`). `details` is opaque; the kernel
 * stores it and does not interpret it.
 *
 * Intentionally not in the Log: file descriptors, the torn-tail flag, and
 * synthesized tool results (callers append a result themselves if they want
 * it in history). The view projected from these entries is not an entry.
 */

/** Opaque id of one entry, unique within its session. Build with `asEntryId`. */
export type EntryId = string & { readonly __brand: "EntryId" };

/** One entry as stored and read back. `seq` is its 0-based append order. */
export type StoredEntry<K extends string = string, P = unknown> = {
  id: EntryId;
  parentId: EntryId | null;
  seq: number;
  /** ISO-8601 timestamp. */
  at: string;
  kind: K;
  payload: P;
  /** Opaque plugin data. The kernel does not interpret this. */
  details?: Record<string, unknown>;
};

/** What a caller passes to `LogStore.append`; the store fills in the rest. */
export type EntryInput<K extends string = string, P = unknown> = {
  kind: K;
  payload: P;
  details?: Record<string, unknown>;
  /** Omit to hang the entry off the current leaf. */
  parentId?: EntryId;
};

/** Brands a string as an `EntryId`. Does not check that the entry exists. */
export function asEntryId(id: string): EntryId {
  return id as EntryId;
}
