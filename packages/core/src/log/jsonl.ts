/**
 * JSONL log. One file per session: `<dir>/<sessionId>.jsonl`.
 *
 * Guarantees: append writes the whole line on a cached append fd, looping
 * over short writes, and only then commits the entry to memory, so the file
 * is a superset of memory. A failed append truncates the bytes of its own
 * partial line off the file before rethrowing, so later appends start on a
 * clean line; if that truncation fails, the session refuses further appends
 * until a new store reopens the file and drops the partial line as torn. A
 * killed process loses nothing it appended; the store never calls fsync, so
 * an OS crash or power loss can drop or corrupt the tail; when the damage is
 * not confined to the last line, reopen throws `LogCorruptedError`. On open,
 * a JSON parse failure on the last line is discarded, recorded as torn, and
 * truncated off the file before the next append so the new line is not glued
 * to the partial one. A parse failure
 * on any earlier line throws `LogCorruptedError`. An unregistered kind
 * throws `UnknownEntryKindError` and is not skipped. There is no high-water
 * mark (no persisted "written up to here" counter) anywhere in the Log: the
 * next seq is the number of entries loaded.
 *
 * Intentionally not in the Log: the torn flag, the unwritable flag, the cached
 * fd, and this index.
 * Torn does not participate in seq assignment.
 */

import {
  closeSync,
  existsSync,
  fstatSync,
  ftruncateSync,
  mkdirSync,
  openSync,
  readFileSync,
  writeSync,
} from "node:fs";
import path from "node:path";
import { isRecord } from "@sage/provider/contract";
import { LogCorruptedError, UnknownEntryKindError } from "./errors.js";
import "./kinds.js";
import { kindDef } from "./registry.js";
import { LogSession, prepareEntry } from "./session.js";
import type { LogStore } from "./store.js";
import {
  asEntryId,
  type EntryId,
  type EntryInput,
  type StoredEntry,
} from "./types.js";

/** The `node:fs` calls the store makes. Test seam; production uses `node:fs`. */
export type LogFs = {
  mkdirSync(path: string, options?: { recursive?: boolean }): void;
  existsSync(path: string): boolean;
  readFileSync(path: string, encoding: "utf8"): string;
  openSync(path: string, flags: string): number;
  /** Writes from the start of `data` and returns the bytes written. */
  writeSync(fd: number, data: Uint8Array): number;
  ftruncateSync(fd: number, len: number): void;
  fstatSync(fd: number): { size: number };
  closeSync(fd: number): void;
};

/** The production `LogFs`: each method goes straight to `node:fs`. Not re-exported from the package; `@sage/core/testing` builds doubles on it. */
export const nodeFs: LogFs = {
  mkdirSync: (dir, options) => {
    mkdirSync(dir, options);
  },
  existsSync,
  readFileSync: (file, encoding) => readFileSync(file, encoding),
  openSync,
  writeSync: (fd, data) => writeSync(fd, data),
  ftruncateSync,
  fstatSync,
  closeSync,
};

/**
 * Durable `LogStore`: one JSONL file per session under `dir`, created on
 * construction. A session file is read on first use of that session id.
 * Throws on a session id that is empty or contains a path separator or `..`.
 */
export class JsonlLogStore implements LogStore {
  private readonly dir: string;
  private readonly fs: LogFs;
  private readonly sessions = new Map<string, LogSession>();
  private readonly fds = new Map<string, number>();
  private readonly tornSessions = new Set<string>();
  /**
   * Sessions whose partial line from a failed append could not be removed,
   * mapped to the error that stopped the removal.
   */
  private readonly unwritableSessions = new Map<string, unknown>();

  constructor(options: { dir: string; fs?: LogFs }) {
    this.dir = options.dir;
    this.fs = options.fs ?? nodeFs;
    this.fs.mkdirSync(this.dir, { recursive: true });
  }

  /** True when the last line was discarded as a torn write. Not a watermark. */
  torn(sessionId: string): boolean {
    this.session(sessionId);
    return this.tornSessions.has(sessionId);
  }

  /**
   * Persists the entry, then commits it to memory. Throws the write error
   * when the line cannot be written, leaving memory, the leaf, and the file
   * as they were. Throws without writing when an earlier failed append left a
   * partial line it could not remove.
   */
  append(sessionId: string, input: EntryInput): StoredEntry {
    const session = this.session(sessionId);
    const entry = prepareEntry(session, input);
    this.persist(sessionId, entry);
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

  /**
   * Writes the whole line, looping over short writes; a write that makes no
   * progress throws. On any failure the bytes of this line already written
   * are truncated off the end of the file. A failed truncation marks the
   * session unwritable.
   */
  private persist(sessionId: string, entry: StoredEntry): void {
    if (this.unwritableSessions.has(sessionId)) {
      throw new Error(
        `Log session ${sessionId} has a partial line that could not be removed; reopen the store to append`,
        { cause: this.unwritableSessions.get(sessionId) },
      );
    }
    const fd = this.fd(sessionId);
    const line = Buffer.from(`${JSON.stringify(entry)}\n`);
    let written = 0;
    try {
      while (written < line.length) {
        const wrote = this.fs.writeSync(fd, line.subarray(written));
        if (wrote <= 0) throw new Error("log write made no progress");
        written += wrote;
      }
    } catch (err) {
      this.removePartialLine(sessionId, fd, written);
      throw err;
    }
  }

  /**
   * Cuts `written` bytes off the end of the file. The fd appends, so those
   * are this line's bytes even when another writer appended before them.
   */
  private removePartialLine(
    sessionId: string,
    fd: number,
    written: number,
  ): void {
    if (written === 0) return;
    try {
      this.fs.ftruncateSync(fd, this.fs.fstatSync(fd).size - written);
    } catch (err) {
      // Swallows the truncate error so the caller sees the write error that
      // caused it; it resurfaces as the cause of every later append's error.
      // The partial line stays the last line, which the next open discards
      // as torn.
      this.unwritableSessions.set(sessionId, err);
    }
  }

  private fd(sessionId: string): number {
    const cached = this.fds.get(sessionId);
    if (cached !== undefined) return cached;
    const opened = this.fs.openSync(this.file(sessionId), "a");
    this.fds.set(sessionId, opened);
    return opened;
  }

  private session(sessionId: string): LogSession {
    const existing = this.sessions.get(sessionId);
    if (existing) return existing;
    const session = new LogSession();
    const file = this.file(sessionId);
    if (this.fs.existsSync(file)) {
      const text = this.fs.readFileSync(file, "utf8");
      const loaded = parseJsonl(text);
      if (loaded.torn) {
        if (!text.startsWith(loaded.keptText)) throw new LogCorruptedError(1);
        this.tornSessions.add(sessionId);
        this.trimFile(file, Buffer.byteLength(loaded.keptText));
      }
      for (const entry of loaded.entries) session.commit(entry);
    }
    this.sessions.set(sessionId, session);
    return session;
  }

  private trimFile(file: string, bytes: number): void {
    const fd = this.fs.openSync(file, "r+");
    try {
      this.fs.ftruncateSync(fd, bytes);
    } finally {
      this.fs.closeSync(fd);
    }
  }

  private file(sessionId: string): string {
    if (
      sessionId.length === 0 ||
      sessionId.includes("/") ||
      sessionId.includes("\\") ||
      sessionId.includes("..")
    ) {
      throw new Error(`Invalid session id: ${sessionId}`);
    }
    return path.join(this.dir, `${sessionId}.jsonl`);
  }
}

function parseJsonl(text: string): {
  entries: StoredEntry[];
  torn: boolean;
  keptText: string;
} {
  if (text.length === 0) return { entries: [], torn: false, keptText: "" };
  const raw = text.split("\n");
  const lines = text.endsWith("\n") ? raw.slice(0, -1) : raw;
  const entries: StoredEntry[] = [];
  let torn = false;

  for (let i = 0; i < lines.length; i++) {
    const lineNumber = i + 1;
    const isLast = i === lines.length - 1;
    const line = lines[i] ?? "";
    let parsed: unknown;
    try {
      parsed = JSON.parse(line);
    } catch {
      if (isLast) {
        torn = true;
        continue;
      }
      throw new LogCorruptedError(lineNumber);
    }
    const entry = decodeEntry(parsed, entries, lineNumber);
    if (entry.seq !== entries.length) throw new LogCorruptedError(lineNumber);
    entries.push(entry);
  }

  const keptText = torn
    ? lines.length <= 1
      ? ""
      : `${lines.slice(0, -1).join("\n")}\n`
    : text;
  return { entries, torn, keptText };
}

function decodeEntry(
  parsed: unknown,
  prior: readonly StoredEntry[],
  line: number,
): StoredEntry {
  if (!isRecord(parsed)) throw new LogCorruptedError(line);
  if (typeof parsed.kind !== "string") throw new LogCorruptedError(line);
  const def = kindDef(parsed.kind);
  if (!def) throw new UnknownEntryKindError(parsed.kind);
  if (!def.validate(parsed.payload)) throw new LogCorruptedError(line);
  if (typeof parsed.id !== "string" || parsed.id.length === 0) {
    throw new LogCorruptedError(line);
  }
  if (parsed.parentId !== null && typeof parsed.parentId !== "string") {
    throw new LogCorruptedError(line);
  }
  if (typeof parsed.seq !== "number" || !Number.isInteger(parsed.seq)) {
    throw new LogCorruptedError(line);
  }
  if (typeof parsed.at !== "string") throw new LogCorruptedError(line);

  const parentId = parsed.parentId === null ? null : asEntryId(parsed.parentId);
  if (parentId !== null && !prior.some((entry) => entry.id === parentId)) {
    throw new LogCorruptedError(line);
  }
  if (parsed.kind === "leaf") {
    const target = isRecord(parsed.payload) ? parsed.payload.target : undefined;
    if (
      typeof target !== "string" ||
      !prior.some((entry) => entry.id === target)
    ) {
      throw new LogCorruptedError(line);
    }
  }

  const entry: StoredEntry = {
    id: asEntryId(parsed.id),
    parentId,
    seq: parsed.seq,
    at: parsed.at,
    kind: parsed.kind,
    payload: parsed.payload,
  };
  if (parsed.details !== undefined) {
    if (!isRecord(parsed.details)) throw new LogCorruptedError(line);
    entry.details = parsed.details;
  }
  return entry;
}
