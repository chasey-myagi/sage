/**
 * JSONL log. One file per session: `<dir>/<sessionId>.jsonl`.
 *
 * Contract: append writes the line with `writeSync` on a cached append fd, then
 * `fsyncSync`, and only then commits the entry to memory. Disk is a superset of
 * memory. On open, a JSON parse failure on the last line is discarded,
 * recorded as torn, and truncated off the file before the next append so the
 * new line is not glued to the partial one. A parse failure on any earlier
 * line throws `LogCorruptedError`. An unregistered kind throws
 * `UnknownEntryKindError` and is not skipped. There is no high-water mark; the
 * next seq is the number of entries loaded.
 *
 * Intentionally not in the Log: the torn flag, the cached fd, and this index.
 * Torn does not participate in seq assignment.
 */

import {
  closeSync,
  existsSync,
  fsyncSync,
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

export type LogFs = {
  mkdirSync(path: string, options?: { recursive?: boolean }): void;
  existsSync(path: string): boolean;
  readFileSync(path: string, encoding: "utf8"): string;
  openSync(path: string, flags: string): number;
  writeSync(fd: number, data: string): number;
  fsyncSync(fd: number): void;
  ftruncateSync(fd: number, len: number): void;
  closeSync(fd: number): void;
};

const nodeFs: LogFs = {
  mkdirSync: (dir, options) => {
    mkdirSync(dir, options);
  },
  existsSync,
  readFileSync: (file, encoding) => readFileSync(file, encoding),
  openSync,
  writeSync: (fd, data) => writeSync(fd, data),
  fsyncSync,
  ftruncateSync,
  closeSync,
};

export class JsonlLogStore implements LogStore {
  private readonly dir: string;
  private readonly fs: LogFs;
  private readonly sessions = new Map<string, LogSession>();
  private readonly fds = new Map<string, number>();
  private readonly tornSessions = new Set<string>();

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

  private persist(sessionId: string, entry: StoredEntry): void {
    const fd = this.fd(sessionId);
    this.fs.writeSync(fd, `${JSON.stringify(entry)}\n`);
    this.fs.fsyncSync(fd);
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
      this.fs.fsyncSync(fd);
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
