/**
 * In-memory log. Same tree rules as the JSONL store, with no disk.
 *
 * Contract: `append` validates, then commits. A failed validation leaves the
 * session unchanged.
 *
 * Intentionally not in the Log: this map of sessions.
 */

import "./kinds.js";
import { LogSession, prepareEntry } from "./session.js";
import type { LogStore } from "./store.js";
import type { EntryId, EntryInput, StoredEntry } from "./types.js";

export class MemoryLogStore implements LogStore {
  private readonly sessions = new Map<string, LogSession>();

  append(sessionId: string, input: EntryInput): StoredEntry {
    const session = this.session(sessionId);
    const entry = prepareEntry(session, input);
    session.commit(entry);
    return entry;
  }

  leaf(sessionId: string): EntryId | null {
    return this.session(sessionId).leaf();
  }

  get(sessionId: string, id: EntryId): StoredEntry | undefined {
    return this.session(sessionId).get(id);
  }

  path(sessionId: string, leafId?: EntryId): StoredEntry[] {
    return this.session(sessionId).path(leafId);
  }

  all(sessionId: string): readonly StoredEntry[] {
    return this.session(sessionId).all();
  }

  private session(sessionId: string): LogSession {
    let session = this.sessions.get(sessionId);
    if (!session) {
      session = new LogSession();
      this.sessions.set(sessionId, session);
    }
    return session;
  }
}
