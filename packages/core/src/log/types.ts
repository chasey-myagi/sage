/**
 * Log entry shapes.
 *
 * Contract: a stored entry is pure JSON (`id`, `parentId`, `seq`, `at`, `kind`,
 * `payload`, optional `details`). `details` is opaque; the kernel stores it and
 * does not interpret it.
 *
 * Intentionally not in the Log: high-water marks, file descriptors, the torn-tail
 * flag, and synthesized tool results. Callers append a result themselves if they
 * want it in history. View projection is not a log entry (that is #5).
 */

export type EntryId = string & { readonly __brand: "EntryId" };

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

export type EntryInput<K extends string = string, P = unknown> = {
  kind: K;
  payload: P;
  details?: Record<string, unknown>;
  /** Omit to hang the entry off the current leaf. */
  parentId?: EntryId;
};

/**
 * Structural stand-in for `Message` from `@sage/provider/contract`.
 * That package does not export `Message` on this branch (#3). Log checks the
 * `{ role, content }` shape only. No value import from provider.
 */
export type MessagePayload = {
  role: string;
  content: unknown;
};

/**
 * Structural stand-in for `ModelRef` from `@sage/provider/contract` (#3).
 */
export type ModelRef = {
  provider: string;
  model: string;
};

export function asEntryId(id: string): EntryId {
  return id as EntryId;
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
