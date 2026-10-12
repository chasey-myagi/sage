/**
 * Import-time entry kind registry.
 *
 * Guarantees: the kernel only appends, replays, and reads the leaf. Each kind
 * registers its own validator and optional pure reducer. Registering the same
 * kind twice throws. Reading an unregistered kind refuses the log.
 *
 * Intentionally not in the Log: the registry itself. It is process state, not
 * history. Reducers do not receive the store.
 */

import type { StoredEntry } from "./types.js";

/**
 * One entry kind. `validate` runs on every append and every reopen;
 * `reduce`, when present, must be pure. `doc` is copied into
 * docs/log-manifest.md.
 */
type EntryKindDef<P, S = unknown> = {
  kind: string;
  persisted: true;
  validate: (payload: unknown) => payload is P;
  reduce?: (state: S, entry: StoredEntry<string, P>) => S;
  doc: string;
};

const kinds: EntryKindDef<unknown>[] = [];
const byKind = new Map<string, EntryKindDef<unknown>>();

/** Registers a kind at import time. Throws when the kind already exists. */
export function registerEntryKind<P, S = unknown>(
  def: EntryKindDef<P, S>,
): void {
  if (byKind.has(def.kind)) {
    throw new Error(`Entry kind already registered: ${def.kind}`);
  }
  const stored = def as EntryKindDef<unknown>;
  kinds.push(stored);
  byKind.set(def.kind, stored);
}

/** Every registered kind, in registration order. */
export function entryKinds(): readonly EntryKindDef<unknown>[] {
  return kinds;
}

/** The definition for `kind`, or undefined when unregistered. */
export function kindDef(kind: string): EntryKindDef<unknown> | undefined {
  return byKind.get(kind);
}
