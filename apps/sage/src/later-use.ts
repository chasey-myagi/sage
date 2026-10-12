/**
 * Offline later-use labels: whether a tool chunk was needed after a
 * decision about it, computed from one Log path. `sage stats` audits
 * shadow `none` decisions with them; `sage calibrate-jev` sweeps against
 * them.
 *
 * Contract: `laterUseIn` is a pure function of the entries; it reads
 * message entries and manifest `turn` only. Two labels come out together:
 * `touch` (the path was touched again) and `content` (text the chunk
 * introduced was used). Both are true for an `expand` of the chunk. The
 * package README defines both and their known biases.
 *
 * Intentionally not in the Log: the labels. They are after-the-fact
 * judgments over what the session already wrote; nothing reads them back.
 */

import { isFiniteNumber, isRecord, type StoredEntry } from "@sage/core";
import {
  bashSearchPatterns,
  EXPAND_TOOL_NAME,
  normalizeBashCommand,
  toolUseIdOfChunk,
} from "@sage/plugins";
import { TOOL_NAMES } from "@sage/tools";

/**
 * Whether a chunk was needed after a decision. `touch`: it was expanded,
 * or it is a `read` result whose `path` is read again or edited on a
 * strictly later turn. `content`: it was expanded, a later call used a
 * string its output introduced, or it is a `bash` chunk whose command
 * runs again.
 */
export type LaterUse = { touch: boolean; content: boolean };

/**
 * The labels of chunk `chunkId` decided at path index `at` on manifest
 * turn `turn`. Calls count when their assistant entry comes after `at`;
 * touch also needs them on a turn after `turn`.
 */
export type LaterUseOf = (
  chunkId: string,
  at: number,
  turn: number,
) => LaterUse;

/** A label name, as `calibrate-jev --label` takes it. */
export type LaterUseLabel = keyof LaterUse;

/** The labels `calibrate-jev --label` accepts. */
export const LATER_USE_LABELS = [
  "touch",
  "content",
] as const satisfies readonly LaterUseLabel[];

/**
 * Strings shorter than this many characters are never credited to a
 * chunk: a one- or two-character pattern or path matches almost any
 * output by chance.
 */
const MIN_INTRODUCED_CHARS = 3;

/**
 * Joins corpus segments so no match spans two of them. A query that
 * itself contains U+0000 is skipped.
 */
const SEGMENT_SEPARATOR = "\u0000";

type ToolCall = {
  /** Position of the assistant entry in the Log. */
  index: number;
  turn: number;
  name: string;
  toolUseId: string;
  input: unknown;
  /** Corpus offset where the call's assistant message starts. */
  messageStart: number;
};

/**
 * Every message text on the path joined in Log order. A segment whose
 * `toolUseId` is set is that call's tool_result content; other segments
 * are user and assistant text and the string values of tool inputs.
 */
type Corpus = {
  text: string;
  starts: number[];
  toolUseIds: (string | undefined)[];
};

/**
 * Builds both labels over `entries` once; each lookup afterwards is a few
 * map reads plus the touch scan over the calls.
 *
 * Content credit, per call made after the chunk: each query string of the
 * call (an `edit`'s `old_string`, a grep tool's `pattern`, the patterns of
 * grep and rg stages in a `bash` command, the `path` of a `read`, `edit`,
 * or `write` with a leading `./` removed) is located at its first
 * occurrence in the corpus. When that occurrence lies in a tool_result
 * before the call's own message, the result's chunk is credited with the
 * call. A string the task, an earlier input, or an earlier output already
 * held is credited to nobody else, so a re-read never takes credit from
 * the read that first showed the text.
 */
export function laterUseIn(entries: readonly StoredEntry[]): LaterUseOf {
  const { calls, corpus } = scanPath(entries);
  const byToolUseId = new Map(calls.map((call) => [call.toolUseId, call]));
  const lastExpand = lastExpands(calls);
  const lastCredit = creditedCalls(calls, corpus);
  const runs = bashRuns(calls);
  return (chunkId, at, turn) => {
    const toolUseId = toolUseIdOfChunk(chunkId);
    const origin = toolUseId ? byToolUseId.get(toolUseId) : undefined;
    const expanded = (lastExpand.get(chunkId) ?? -1) > at;
    const touched =
      origin !== undefined && origin.name === TOOL_NAMES.read
        ? touchedLater(origin, turn, calls)
        : false;
    const credited =
      toolUseId !== undefined && (lastCredit.get(toolUseId) ?? -1) > at;
    return {
      touch: expanded || touched,
      content:
        expanded ||
        credited ||
        (origin !== undefined && ranAgain(origin, at, runs)),
    };
  };
}

/** The tool calls and the corpus of one path, in one pass. */
function scanPath(entries: readonly StoredEntry[]): {
  calls: ToolCall[];
  corpus: Corpus;
} {
  const calls: ToolCall[] = [];
  const parts: string[] = [];
  const corpus: Corpus = { text: "", starts: [], toolUseIds: [] };
  let length = 0;
  const push = (text: string, toolUseId?: string) => {
    corpus.starts.push(length);
    corpus.toolUseIds.push(toolUseId);
    parts.push(text);
    length += text.length + SEGMENT_SEPARATOR.length;
  };
  let turn = 0;
  entries.forEach((entry, index) => {
    if (entry.kind === "manifest" && isRecord(entry.payload)) {
      if (isFiniteNumber(entry.payload.turn)) turn = entry.payload.turn;
      return;
    }
    if (entry.kind !== "message" || !isMessage(entry.payload)) return;
    const messageStart = length;
    for (const block of entry.payload.content) {
      if (!isRecord(block)) continue;
      if (block.type === "tool_result") {
        if (typeof block.toolUseId === "string") {
          push(stringOf(block.content), block.toolUseId);
        }
        continue;
      }
      if (block.type === "tool_use") {
        const call = toolCall(block, { index, turn, messageStart });
        if (call) calls.push(call);
        for (const text of stringLeaves(block.input)) push(text);
        continue;
      }
      if (typeof block.text === "string") push(block.text);
    }
  });
  corpus.text = parts.join(SEGMENT_SEPARATOR);
  return { calls, corpus };
}

function toolCall(
  block: Record<string, unknown>,
  at: { index: number; turn: number; messageStart: number },
): ToolCall | undefined {
  if (typeof block.name !== "string" || typeof block.id !== "string") {
    return undefined;
  }
  return { ...at, name: block.name, toolUseId: block.id, input: block.input };
}

/** The latest `expand` call index per chunk id it names. */
function lastExpands(calls: readonly ToolCall[]): Map<string, number> {
  const last = new Map<string, number>();
  for (const call of calls) {
    if (call.name !== EXPAND_TOOL_NAME) continue;
    const id = nonEmptyString(call.input, "id");
    if (id) last.set(id, call.index);
  }
  return last;
}

/**
 * The latest crediting call index per tool_use id whose result introduced
 * a string a later call used. One `indexOf` per distinct query string,
 * cached: the first occurrence does not depend on which call asks. An
 * untransformed query is in its own call's input, so its scan stops there
 * at the latest.
 */
function creditedCalls(
  calls: readonly ToolCall[],
  corpus: Corpus,
): Map<string, number> {
  const first = new Map<string, number>();
  const last = new Map<string, number>();
  for (const call of calls) {
    for (const query of queryStrings(call)) {
      if (query.length < MIN_INTRODUCED_CHARS) continue;
      if (query.includes(SEGMENT_SEPARATOR)) continue;
      let offset = first.get(query);
      if (offset === undefined) {
        offset = corpus.text.indexOf(query);
        first.set(query, offset);
      }
      if (offset < 0 || offset >= call.messageStart) continue;
      const owner = corpus.toolUseIds[segmentAt(corpus.starts, offset)];
      if (owner !== undefined) {
        last.set(owner, Math.max(last.get(owner) ?? -1, call.index));
      }
    }
  }
  return last;
}

/** The strings a call takes from earlier context, per the label's rules. */
function queryStrings(call: ToolCall): string[] {
  const out: string[] = [];
  if (call.name === TOOL_NAMES.edit) {
    const old = nonEmptyString(call.input, "old_string");
    if (old) out.push(old);
  }
  if (call.name === TOOL_NAMES.grep) {
    const pattern = nonEmptyString(call.input, "pattern");
    if (pattern) out.push(literalOf(pattern));
  }
  if (call.name === TOOL_NAMES.bash) {
    const command = nonEmptyString(call.input, "command");
    if (command) out.push(...bashSearchPatterns(command).map(literalOf));
  }
  if (OPENING_TOOLS.has(call.name)) {
    const file = nonEmptyString(call.input, "path");
    if (file) out.push(file.replace(/^(?:\.\/)+/, ""));
  }
  return out;
}

/** Tools whose `path` input opens a file a chunk may have named. */
const OPENING_TOOLS: ReadonlySet<string> = new Set([
  TOOL_NAMES.read,
  TOOL_NAMES.edit,
  TOOL_NAMES.write,
]);

/**
 * A regular expression read as the literal text it most likely came from:
 * a backslash before punctuation (`foo\(`, `a\.b`) is dropped. Other
 * metacharacters are kept, so such a pattern rarely matches.
 */
function literalOf(pattern: string): string {
  return pattern.replace(/\\([^\w\s])/g, "$1");
}

/** Index of the segment containing corpus `offset` (binary search). */
function segmentAt(starts: readonly number[], offset: number): number {
  let low = 0;
  let high = starts.length - 1;
  while (low < high) {
    const mid = (low + high + 1) >> 1;
    if ((starts[mid] ?? 0) <= offset) low = mid;
    else high = mid - 1;
  }
  return low;
}

/** Bash calls by normalized command, in Log order. */
function bashRuns(calls: readonly ToolCall[]): Map<string, ToolCall[]> {
  const runs = new Map<string, ToolCall[]>();
  for (const call of calls) {
    if (call.name !== TOOL_NAMES.bash) continue;
    const command = nonEmptyString(call.input, "command");
    if (!command) continue;
    const key = normalizeBashCommand(command);
    const same = runs.get(key);
    if (same) same.push(call);
    else runs.set(key, [call]);
  }
  return runs;
}

function ranAgain(
  origin: ToolCall,
  at: number,
  runs: ReadonlyMap<string, readonly ToolCall[]>,
): boolean {
  if (origin.name !== TOOL_NAMES.bash) return false;
  const command = nonEmptyString(origin.input, "command");
  if (!command) return false;
  const same = runs.get(normalizeBashCommand(command)) ?? [];
  return same.some(
    (call) => call.toolUseId !== origin.toolUseId && call.index > at,
  );
}

/**
 * True when a `read` or `edit` other than `origin` names `origin`'s
 * `path` on a turn after `judgedTurn`.
 */
function touchedLater(
  origin: ToolCall,
  judgedTurn: number,
  calls: readonly ToolCall[],
): boolean {
  const file = nonEmptyString(origin.input, "path");
  if (!file) return false;
  return calls.some(
    (call) =>
      (call.name === TOOL_NAMES.read || call.name === TOOL_NAMES.edit) &&
      call.turn > judgedTurn &&
      call.toolUseId !== origin.toolUseId &&
      nonEmptyString(call.input, "path") === file,
  );
}

/** Every string value inside a tool input, depth first. */
function stringLeaves(value: unknown): string[] {
  if (typeof value === "string") return [value];
  if (Array.isArray(value)) return value.flatMap(stringLeaves);
  if (isRecord(value)) return Object.values(value).flatMap(stringLeaves);
  return [];
}

function stringOf(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function nonEmptyString(input: unknown, key: string): string | undefined {
  if (!isRecord(input)) return undefined;
  const value = input[key];
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function isMessage(value: unknown): value is { content: unknown[] } {
  return isRecord(value) && Array.isArray(value.content);
}
