/**
 * Offline later-use labels: whether a tool chunk was needed after a
 * decision about it, computed from one Log path. `sage stats` audits
 * shadow `none` decisions with them; `sage calibrate-jev` sweeps against
 * them.
 *
 * Contract: `laterUseIn` is a pure function of the entries; it reads
 * message entries, manifest `turn`, and the `Workspace:` line of
 * `request_header.system`. Two labels come out together: `touch` (the
 * path was touched again) and `content` (text the chunk showed was used).
 * Both are true for an `expand` of the chunk. The package README defines
 * both and their known biases.
 *
 * Intentionally not in the Log: the labels. They are after-the-fact
 * judgments over what the session already wrote; nothing reads them back.
 */

import { isFiniteNumber, isRecord, type StoredEntry } from "@sage/core";
import {
  bashFileOperands,
  bashSearchPatterns,
  EXPAND_TOOL_NAME,
  normalizeBashCommand,
  toolUseIdOfChunk,
} from "@sage/plugins";
import { TOOL_NAMES } from "@sage/tools";
import { WORKSPACE_LINE_PREFIX } from "./coding-session.js";

/**
 * Whether a chunk was needed after a decision. `touch`: it was expanded,
 * or it is a `read` result whose `path` is read again or edited on a
 * strictly later turn. `content`: it was expanded, a later call used a
 * string its output supplied, or it is a `bash` chunk whose command runs
 * again.
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
const MIN_CREDITED_CHARS = 3;

/**
 * A path suffix used as the last fallback keeps at least this many
 * segments: `src/a.ts`, never a bare `a.ts`.
 */
const MIN_SUFFIX_SEGMENTS = 2;

/**
 * Joins corpus pieces so no match spans two of them. A query that itself
 * contains U+0000 is skipped.
 */
const PIECE_SEPARATOR = "\u0000";

type ToolCall = {
  /** Position of the assistant entry in the Log. */
  index: number;
  turn: number;
  name: string;
  toolUseId: string;
  input: unknown;
  /** Corpus offset where the call's assistant message starts. */
  messageStart: number;
  /** Workspace root of the latest `request_header` before the call. */
  root: string | undefined;
};

/**
 * Every message text on the path joined in Log order: user and assistant
 * text, thinking, the string values of tool inputs, and tool results. A
 * piece whose `toolUseIds` entry is set is that call's tool_result.
 */
type Corpus = {
  text: string;
  starts: number[];
  toolUseIds: (string | undefined)[];
};

/**
 * One string a call took from earlier context. `forms` are tried in order
 * until one has a copy before the call. `copy` picks which copy decides:
 * `nearest` (the last one before the call) for text copied verbatim,
 * `first` (where it first appeared) for names.
 */
type Query = { forms: string[]; copy: "nearest" | "first" };

/**
 * Builds both labels over `entries` once; each lookup afterwards is a few
 * map reads plus the touch scan over the calls.
 *
 * Content credit, per call: each query string of the call is located at
 * one copy before the call's own message. When that copy lies in a
 * tool_result, the result's chunk is credited with the call; when it lies
 * in user or assistant text or in a tool input, nobody is. An `edit`'s
 * `old_string` uses its nearest copy: the text is copied from the latest
 * view of the file. Names use their first copy, so a later listing or
 * re-read never takes credit for a name an earlier chunk or the task
 * already gave: a grep tool's `pattern`, the patterns of grep and rg
 * stages in `bash`, the `path` of `read`, `edit`, and `write`, and the
 * file operands of bash reader stages.
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
    length += text.length + PIECE_SEPARATOR.length;
  };
  let turn = 0;
  let root: string | undefined;
  entries.forEach((entry, index) => {
    if (!isRecord(entry.payload)) return;
    if (entry.kind === "manifest") {
      if (isFiniteNumber(entry.payload.turn)) turn = entry.payload.turn;
      return;
    }
    if (entry.kind === "request_header") {
      const system = entry.payload.system;
      if (typeof system === "string") root = workspaceRoot(system) ?? root;
      return;
    }
    if (entry.kind !== "message" || !isMessage(entry.payload)) return;
    const at = { index, turn, messageStart: length, root };
    for (const block of entry.payload.content) {
      if (!isRecord(block)) continue;
      if (block.type === "tool_result") {
        if (typeof block.toolUseId === "string") {
          push(stringOf(block.content), block.toolUseId);
        }
        continue;
      }
      if (block.type === "tool_use") {
        const call = toolCall(block, at);
        if (call) calls.push(call);
        for (const text of stringLeaves(block.input)) push(text);
        continue;
      }
      if (typeof block.text === "string") push(block.text);
    }
  });
  corpus.text = parts.join(PIECE_SEPARATOR);
  return { calls, corpus };
}

/**
 * The path after the last `Workspace: ` line of a system prompt, the
 * line `systemPrompt` in `coding-session.ts` writes; undefined without
 * one.
 */
function workspaceRoot(system: string): string | undefined {
  const line = system
    .split("\n")
    .filter((text) => text.startsWith(WORKSPACE_LINE_PREFIX))
    .at(-1);
  const value = line?.slice(WORKSPACE_LINE_PREFIX.length).trim();
  return value ? value.replace(/\/+$/, "") : undefined;
}

function toolCall(
  block: Record<string, unknown>,
  at: Omit<ToolCall, "name" | "toolUseId" | "input">,
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
 * The latest crediting call index per tool_use id. A `first` form costs
 * one `indexOf`, cached per distinct form because the first copy does not
 * depend on the call. A `nearest` form costs one backward `lastIndexOf`
 * from the call's message start, which stops at the nearest copy.
 */
function creditedCalls(
  calls: readonly ToolCall[],
  corpus: Corpus,
): Map<string, number> {
  const last = new Map<string, number>();
  const firstCopy = new Map<string, number>();
  const locate = (form: string, query: Query, before: number): number => {
    if (query.copy === "nearest") {
      const from = before - form.length - PIECE_SEPARATOR.length;
      return from < 0 ? -1 : corpus.text.lastIndexOf(form, from);
    }
    let offset = firstCopy.get(form);
    if (offset === undefined) {
      offset = corpus.text.indexOf(form);
      firstCopy.set(form, offset);
    }
    return offset < before ? offset : -1;
  };
  for (const call of calls) {
    for (const query of queriesOf(call)) {
      const owner = sourceOf(query, call.messageStart, corpus, locate);
      if (owner !== undefined) {
        last.set(owner, Math.max(last.get(owner) ?? -1, call.index));
      }
    }
  }
  return last;
}

/**
 * The tool_use id whose result holds the deciding copy of the first form
 * of `query` that has a copy before `before`; undefined when that copy is
 * not in a tool_result or no form has one. `locate` returns the copy's
 * corpus offset, or -1.
 */
function sourceOf(
  query: Query,
  before: number,
  corpus: Corpus,
  locate: (form: string, query: Query, before: number) => number,
): string | undefined {
  for (const form of query.forms) {
    if (form.length < MIN_CREDITED_CHARS) continue;
    if (form.includes(PIECE_SEPARATOR)) continue;
    const offset = locate(form, query, before);
    if (offset >= 0) return corpus.toolUseIds[pieceAt(corpus.starts, offset)];
  }
  return undefined;
}

/** The strings a call took from earlier context, per the label's rules. */
function queriesOf(call: ToolCall): Query[] {
  const out: Query[] = [];
  const named = (forms: string[]) => out.push({ forms, copy: "first" });
  if (call.name === TOOL_NAMES.edit) {
    const old = nonEmptyString(call.input, "old_string");
    if (old) out.push({ forms: [old], copy: "nearest" });
  }
  if (call.name === TOOL_NAMES.grep) {
    const pattern = nonEmptyString(call.input, "pattern");
    if (pattern) named([literalOf(pattern)]);
  }
  if (OPENING_TOOLS.has(call.name)) {
    const file = nonEmptyString(call.input, "path");
    if (file) named(pathForms(file, call.root));
  }
  if (call.name === TOOL_NAMES.bash) {
    const command = nonEmptyString(call.input, "command") ?? "";
    for (const pattern of bashSearchPatterns(command)) {
      named([literalOf(pattern)]);
    }
    for (const file of bashFileOperands(command)) {
      named(pathForms(file, call.root));
    }
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
 * The forms a path is looked up under, in order: relative to the
 * workspace `root` when it lies inside it (tools print paths relative to
 * the workspace), as written, then ever shorter suffixes of at least
 * `MIN_SUFFIX_SEGMENTS` segments. A leading `./` is dropped.
 */
function pathForms(raw: string, root: string | undefined): string[] {
  const written = raw.replace(/^(?:\.\/)+/, "");
  const inside = root && written.startsWith(`${root}/`);
  const relative = inside ? written.slice(root.length + 1) : undefined;
  const forms = relative ? [relative, written] : [written];
  const segments = (relative ?? written).split("/").filter(Boolean);
  for (let keep = segments.length - 1; keep >= MIN_SUFFIX_SEGMENTS; keep -= 1) {
    forms.push(segments.slice(-keep).join("/"));
  }
  return forms;
}

/**
 * A regular expression read as the literal text it most likely came from:
 * a backslash before punctuation (`foo\(`, `a\.b`) is dropped. Other
 * metacharacters are kept, so such a pattern rarely matches.
 */
function literalOf(pattern: string): string {
  return pattern.replace(/\\([^\w\s])/g, "$1");
}

/** Index of the piece containing corpus `offset` (binary search). */
function pieceAt(starts: readonly number[], offset: number): number {
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

function isMessage(
  value: unknown,
): value is { role: unknown; content: unknown[] } {
  return isRecord(value) && Array.isArray(value.content);
}
