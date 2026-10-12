/**
 * `sage stats [--turns] <session.jsonl>`: prints a finished session's
 * health and the shadow scorer audit as one JSON line, without starting a
 * run. `--turns` prints the per-turn audit of `stats-turns.ts` instead.
 *
 * Contract: the default output is `runHealth` of the file plus
 * `shadowNone` and, per later-use label, `touch` and `content` objects of
 * `{ laterNeeded, precision }` from {@link shadowAudit}.
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
  isFiniteNumber,
  isRecord,
  JsonlLogStore,
  runHealth,
  type StoredEntry,
} from "@sage/core";
import type { CommandIo } from "./command-io.js";
import { laterUseIn } from "./later-use.js";
import {
  type CommandFlags,
  type OptionName,
  readFlags,
  renderUsage,
} from "./options.js";
import { turnReport } from "./stats-turns.js";

/** How often shadow `none` decisions were later contradicted, per label. */
type ShadowAudit = {
  /** Distinct chunk ids a shadow decision marked `none`. */
  shadowNone: number;
  touch: LabelAudit;
  content: LabelAudit;
};

/** One label's side of the audit. */
type LabelAudit = {
  /** Of the `none` ids, those the label marks needed later. */
  laterNeeded: number;
  /** `1 - laterNeeded / shadowNone`, or null when `shadowNone` is 0. */
  precision: number | null;
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

/** Audits shadow `none` decisions against both labels of {@link laterUseIn}. */
function shadowAudit(entries: readonly StoredEntry[]): ShadowAudit {
  const judgedTurn = new Map<string, number>();
  const judgedAt = new Map<string, number>();
  let currentTurn = 0;
  entries.forEach((entry, index) => {
    if (entry.kind !== "manifest" || !isRecord(entry.payload)) return;
    if (isFiniteNumber(entry.payload.turn)) currentTurn = entry.payload.turn;
    rememberShadow(entry.payload.shadow, index, currentTurn, {
      judgedTurn,
      judgedAt,
    });
  });
  const useOf = laterUseIn(entries);
  const needed = { touch: 0, content: 0 };
  for (const [id, turn] of judgedTurn) {
    const use = useOf(id, judgedAt.get(id) ?? -1, turn);
    if (use.touch) needed.touch += 1;
    if (use.content) needed.content += 1;
  }
  const shadowNone = judgedTurn.size;
  const audit = (laterNeeded: number): LabelAudit => ({
    laterNeeded,
    precision: shadowNone === 0 ? null : 1 - laterNeeded / shadowNone,
  });
  return {
    shadowNone,
    touch: audit(needed.touch),
    content: audit(needed.content),
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
