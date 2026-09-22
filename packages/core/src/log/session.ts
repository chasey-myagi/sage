/**
 * In-memory session tree shared by every store.
 *
 * Contract: entries are append-only. The leaf is derived by scanning entries
 * in seq order: a `leaf` entry sets the pointer to its target, any other kind
 * sets the pointer to itself. `path` walks parent links from that pointer back
 * to the root and omits `leaf` entries. Nothing here is a high-water mark.
 *
 * Intentionally not in the Log: this index. Disk or the caller holds the
 * entries; this object is rebuilt by replaying those entries.
 */

import { randomUUID } from "node:crypto";
import { InvalidEntryError, UnknownEntryKindError } from "./errors.js";
import { kindDef } from "./registry.js";
import {
  asEntryId,
  type EntryId,
  type EntryInput,
  isRecord,
  type StoredEntry,
} from "./types.js";

export class LogSession {
  private readonly entries: StoredEntry[] = [];
  private readonly byId = new Map<string, StoredEntry>();

  all(): readonly StoredEntry[] {
    return this.entries;
  }

  get(id: EntryId): StoredEntry | undefined {
    return this.byId.get(id);
  }

  has(id: string): boolean {
    return this.byId.has(id);
  }

  leaf(): EntryId | null {
    let current: EntryId | null = null;
    for (const entry of this.entries) {
      current = entry.kind === "leaf" ? readTarget(entry.payload) : entry.id;
    }
    return current;
  }

  path(leafId?: EntryId): StoredEntry[] {
    const startId = leafId ?? this.leaf();
    if (startId === null) return [];
    let current = this.byId.get(startId);
    if (!current) throw new Error(`Unknown entry: ${startId}`);
    if (current.kind === "leaf") {
      current = this.byId.get(readTarget(current.payload));
      if (!current) throw new Error(`Unknown entry: ${startId}`);
    }

    const chain: StoredEntry[] = [];
    const seen = new Set<string>();
    while (current) {
      if (seen.has(current.id))
        throw new Error(`Cycle in log at ${current.id}`);
      seen.add(current.id);
      if (current.kind !== "leaf") chain.push(current);
      if (current.parentId === null) break;
      const parent = this.byId.get(current.parentId);
      if (!parent) throw new Error(`Broken parent link: ${current.parentId}`);
      current = parent;
    }
    chain.reverse();
    return chain;
  }

  commit(entry: StoredEntry): void {
    this.entries.push(entry);
    this.byId.set(entry.id, entry);
  }
}

export function prepareEntry(
  session: LogSession,
  input: EntryInput,
): StoredEntry {
  const def = kindDef(input.kind);
  if (!def) throw new UnknownEntryKindError(input.kind);
  if (!def.validate(input.payload)) throw new InvalidEntryError(input.kind);

  const parentId =
    input.parentId !== undefined ? input.parentId : session.leaf();
  if (parentId !== null && !session.has(parentId)) {
    throw new Error(`Unknown parent: ${parentId}`);
  }
  if (input.kind === "leaf" && !session.has(readTarget(input.payload))) {
    throw new InvalidEntryError("leaf");
  }

  const entry: StoredEntry = {
    id: asEntryId(randomUUID()),
    parentId,
    seq: session.all().length,
    at: new Date().toISOString(),
    kind: input.kind,
    payload: input.payload,
  };
  if (input.details !== undefined) entry.details = input.details;
  return entry;
}

function readTarget(payload: unknown): EntryId {
  if (!isRecord(payload) || typeof payload.target !== "string") {
    throw new InvalidEntryError("leaf");
  }
  return asEntryId(payload.target);
}
