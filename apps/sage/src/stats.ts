/**
 * `sage stats [--turns] <session.jsonl>`: prints a finished session's
 * health and the shadow scorer audit as one JSON line, without starting a
 * run. `--turns` prints the per-turn audit of `stats-turns.ts` instead.
 *
 * Contract: the default output is `runHealth` of the file plus
 * `shadowNone`, `laterNeeded`, and `precision` from {@link shadowAudit}.
 * Either output holds counts and ids only, never paths or message text.
 * Exit 0 after printing; exit 1 with the usage for a bad argument, or with
 * one stderr line for a missing or unreadable file.
 *
 * Intentionally not in the Log: the audit. It is an after-the-fact label
 * computed from the Log; the compiler never reads it.
 */

import { existsSync } from "node:fs";
import path from "node:path";
import {
  errorText,
  isRecord,
  JsonlLogStore,
  runHealth,
  type StoredEntry,
} from "@sage/core";
import { TOOL_NAMES } from "@sage/tools";
import type { CommandIo } from "./command-io.js";
import { isFiniteNumber } from "./guards.js";
import {
  type CommandFlags,
  type OptionName,
  readFlags,
  renderUsage,
} from "./options.js";
import { turnReport } from "./stats-turns.js";

/** How often shadow `none` decisions were later contradicted. */
type ShadowAudit = {
  /** Distinct chunk ids a shadow decision marked `none`. */
  shadowNone: number;
  /** Of those, ids the model needed on a later turn. */
  laterNeeded: number;
  /** `1 - laterNeeded / shadowNone`, or null when `shadowNone` is 0. */
  precision: number | null;
};

type ToolCall = {
  /** Position of the assistant entry in the Log. */
  index: number;
  turn: number;
  name: string;
  toolUseId: string;
  input: unknown;
};

/** Flags `sage stats` accepts; the session file is its one positional. */
const STATS_FLAGS = [
  { name: "turns", usage: "optional" },
] as const satisfies CommandFlags<OptionName>;

/** The `sage stats` usage line, without the leading `usage:`. */
export function statsUsage(): string {
  return `${renderUsage("sage stats", STATS_FLAGS)} <session.jsonl>`;
}

/** Runs `sage stats <argv>` and returns the process exit code. */
export function runStats(argv: string[], io: CommandIo): number {
  const parsed = parseStatsArgs(argv);
  if (!parsed) {
    io.stderr.write(`usage: ${statsUsage()}\n`);
    return 1;
  }
  const resolved = path.resolve(io.cwd, parsed.file);
  const sessionId = path.basename(resolved, ".jsonl");
  if (!resolved.endsWith(".jsonl") || sessionId.length === 0) {
    io.stderr.write(`usage: ${statsUsage()}\n`);
    return 1;
  }
  if (!existsSync(resolved)) {
    io.stderr.write(`Missing session file: ${resolved}\n`);
    return 1;
  }
  try {
    const store = new JsonlLogStore({ dir: path.dirname(resolved) });
    const logged = store.path(sessionId);
    const report = parsed.turns
      ? turnReport(logged)
      : { ...runHealth(logged), ...shadowAudit(logged) };
    io.stdout.write(`${JSON.stringify(report)}\n`);
    return 0;
  } catch (error) {
    io.stderr.write(`${errorText(error)}\n`);
    return 1;
  }
}

/** One session file and an optional `--turns`; undefined for anything else. */
function parseStatsArgs(
  argv: readonly string[],
): { file: string; turns: boolean } | undefined {
  let file: string | undefined;
  let turns = false;
  for (const token of readFlags(argv, STATS_FLAGS)) {
    if (token.kind === "switch") {
      turns = true;
      continue;
    }
    if (token.kind !== "positional" || file !== undefined) return undefined;
    file = token.value;
  }
  return file ? { file, turns } : undefined;
}

/**
 * Audits shadow `none` decisions against what happened later. A chunk
 * counts as later needed when its id appears as an `expand` input `id`
 * after the decision, or it is a `read` result whose `path` is read again
 * on a strictly later turn. A re-read on the deciding turn does not count.
 */
function shadowAudit(entries: readonly StoredEntry[]): ShadowAudit {
  const judgedTurn = new Map<string, number>();
  const judgedAt = new Map<string, number>();
  const calls: ToolCall[] = [];
  let currentTurn = 0;
  entries.forEach((entry, index) => {
    if (entry.kind === "manifest" && isRecord(entry.payload)) {
      if (isFiniteNumber(entry.payload.turn)) currentTurn = entry.payload.turn;
      rememberShadow(entry.payload.shadow, index, currentTurn, {
        judgedTurn,
        judgedAt,
      });
      return;
    }
    if (entry.kind === "message") {
      calls.push(...toolCalls(entry.payload, index, currentTurn));
    }
  });
  let laterNeeded = 0;
  for (const [id, turn] of judgedTurn) {
    const at = judgedAt.get(id) ?? -1;
    if (expandedLater(id, at, calls) || rereadLater(id, turn, calls)) {
      laterNeeded += 1;
    }
  }
  const shadowNone = judgedTurn.size;
  return {
    shadowNone,
    laterNeeded,
    precision: shadowNone === 0 ? null : 1 - laterNeeded / shadowNone,
  };
}

/** Records the first `none` decision per chunk id. */
function rememberShadow(
  shadow: unknown,
  index: number,
  turn: number,
  judged: { judgedTurn: Map<string, number>; judgedAt: Map<string, number> },
): void {
  if (!isRecord(shadow) || !Array.isArray(shadow.decisions)) return;
  for (const decision of shadow.decisions) {
    if (!isRecord(decision) || decision.tier !== "none") continue;
    if (typeof decision.id !== "string" || decision.id.length === 0) continue;
    if (judged.judgedTurn.has(decision.id)) continue;
    judged.judgedTurn.set(decision.id, turn);
    judged.judgedAt.set(decision.id, index);
  }
}

function toolCalls(payload: unknown, index: number, turn: number): ToolCall[] {
  if (!isAssistantMessage(payload)) return [];
  return payload.content.flatMap((block) => {
    if (!isRecord(block) || block.type !== "tool_use") return [];
    if (typeof block.name !== "string" || typeof block.id !== "string") {
      return [];
    }
    return [
      {
        index,
        turn,
        name: block.name,
        toolUseId: block.id,
        input: block.input,
      },
    ];
  });
}

function expandedLater(
  chunkId: string,
  at: number,
  calls: readonly ToolCall[],
): boolean {
  return calls.some(
    (call) =>
      call.index > at &&
      call.name === "expand" &&
      nonEmptyString(call.input, "id") === chunkId,
  );
}

function rereadLater(
  chunkId: string,
  judgedTurn: number,
  calls: readonly ToolCall[],
): boolean {
  const toolUseId = toolUseIdOf(chunkId);
  if (!toolUseId) return false;
  const origin = calls.find((call) => call.toolUseId === toolUseId);
  if (origin?.name !== TOOL_NAMES.read) return false;
  const file = nonEmptyString(origin.input, "path");
  if (!file) return false;
  return calls.some(
    (call) =>
      call.name === TOOL_NAMES.read &&
      call.turn > judgedTurn &&
      call.toolUseId !== toolUseId &&
      nonEmptyString(call.input, "path") === file,
  );
}

/** A tool chunk id is `<prefix>#<toolUseId>`. */
function toolUseIdOf(chunkId: string): string | undefined {
  const hash = chunkId.indexOf("#");
  if (hash <= 0 || hash === chunkId.length - 1) return undefined;
  return chunkId.slice(hash + 1);
}

function nonEmptyString(input: unknown, key: string): string | undefined {
  if (!isRecord(input)) return undefined;
  const value = input[key];
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function isAssistantMessage(
  value: unknown,
): value is { role: string; content: unknown[] } {
  return (
    isRecord(value) &&
    value.role === "assistant" &&
    Array.isArray(value.content)
  );
}
