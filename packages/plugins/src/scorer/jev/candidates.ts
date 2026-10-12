/**
 * Candidate lines: the lines of a tool output that a summary may quote.
 * Code picks them; Jev only judges each one (`keep_<i>`).
 *
 * Guarantees: at most `MAX_CANDIDATES` lines, each trimmed, with runs of
 * whitespace collapsed, at most `CANDIDATE_CHAR_LIMIT` characters, and
 * unique. Past the cap the first `HEAD_CANDIDATES` and the rest from the
 * end survive, since failures and summaries tend to sit at the end of
 * command output. The extractor is chosen by tool name alone; this module
 * does not import the tools.
 *
 * Intentionally not in the Log: the lines. They reach the Log only inside
 * a tier-summary note.
 */

import { isFiniteNumber, isRecord } from "@sage/core";
import { clipEnd } from "../../compiler/clip.js";

/** Most candidate lines one chunk offers, so `keep_<i>` questions stay bounded. */
const MAX_CANDIDATES = 24;
/** Candidates kept from the start past the cap; the rest come from the end. */
const HEAD_CANDIDATES = 8;
/** Longest candidate line in characters; longer lines are cut with `…`. */
const CANDIDATE_CHAR_LIMIT = 200;

/** Error, exception, failure, panic, fatal, or traceback wording. */
const FAILURE =
  /(?:error|exception)\b|\b(?:fail(?:ed|ure|s|ing)?|panic(?:ked)?|fatal|traceback)\b/i;
/** A JavaScript `at …` frame or a Python `File "…", line N` frame. */
const STACK_FRAME = /^\s*at\s+\S|^\s*File "[^"]+", line \d+/;
/** `path.ext:line`, as compilers, linters, and test runners print it. */
const PATH_LINE = /[\w./-]+\.\w+:\d+/;
/** A function, class, type, or module declaration in common languages. */
const SIGNATURE =
  /^\s*(?:export\s+)?(?:pub(?:\([\w:]+\))?\s+)?(?:default\s+)?(?:async\s+)?(?:function\*?|class|interface|type|enum|def|fn|struct|trait|impl|module|namespace)\s+\w/;
/** An export statement. */
const EXPORT = /^\s*export\b|^\s*module\.exports\b|^\s*__all__\s*=/;
/**
 * A test-runner tally: a vitest or jest `Tests` / `Test Files` line, or
 * `<n> passed` / `<n> failed`. A per-file `✓ a.test.ts (3 tests)` line is
 * not a tally.
 */
const TEST_SUMMARY = /^\s*(?:Tests|Test Files)\b|\b\d+ (?:passed|failed)\b/;
/** A unified-diff hunk header or a `diff --git` line. */
const DIFF_HEADER = /^@@ .+ @@|^diff --git /;
/** The status line the bash tool puts first. */
const BASH_STATUS = /^\[bash exit code -?\d+\]$/;
/** The footer a cut read, grep, ls, or find result ends with. */
const TRUNCATION_FOOTER = /^\[truncated: /;
/** A grep hit: `path:line: text`. */
const GREP_HIT = /^[^:\s][^:]*:\d+:/;

type Extractor = (lines: readonly string[], input: unknown) => string[];

const EXTRACTORS: Readonly<Record<string, Extractor>> = {
  read: readCandidates,
  grep: (lines) => lines.filter((line) => GREP_HIT.test(line)),
  ls: listingCandidates,
  find: listingCandidates,
  bash: (lines) =>
    lines.filter((line) => BASH_STATUS.test(line) || isNotable(line)),
};

/** One output's candidates, and whether they already carry all of it. */
export type Candidates = {
  lines: string[];
  /**
   * True when every non-empty output line is a candidate, verbatim apart
   * from whitespace, none cut, merged, or capped away. The output then
   * needs no separate head or tail.
   */
  coversOutput: boolean;
};

/**
 * The candidate lines of one tool output. `read` offers declarations and
 * exports as `L<n>: <line>`, numbered from `input.offset` (1 when absent);
 * `grep` offers every hit; `ls` and `find` offer every entry; `bash`
 * offers its status line and notable lines; any other tool offers notable
 * lines. Notable means failure wording, a stack frame, `path:line`, a
 * declaration or export, a test tally, or a diff header.
 */
export function candidateLines(
  tool: string,
  input: unknown,
  lines: readonly string[],
): Candidates {
  const extract = EXTRACTORS[tool] ?? notableCandidates;
  const picked = extract(lines, input);
  const normalized = normalize(picked);
  const capped = capCandidates(normalized.lines);
  const nonEmpty = lines.filter((line) => line.trim().length > 0);
  return {
    lines: capped,
    coversOutput:
      normalized.lossless &&
      capped.length === normalized.lines.length &&
      picked.length === nonEmpty.length &&
      picked.every((line, index) => line === nonEmpty[index]),
  };
}

function readCandidates(lines: readonly string[], input: unknown): string[] {
  const first = readOffset(input);
  const picked: string[] = [];
  lines.forEach((line, index) => {
    if (SIGNATURE.test(line) || EXPORT.test(line)) {
      picked.push(`L${first + index}: ${line.trim()}`);
    }
  });
  return picked;
}

/** The first line number of a read page: a positive integer `offset`, else 1. */
function readOffset(input: unknown): number {
  if (!isRecord(input)) return 1;
  const offset = input.offset;
  return isFiniteNumber(offset) && Number.isInteger(offset) && offset > 0
    ? offset
    : 1;
}

function listingCandidates(lines: readonly string[]): string[] {
  return lines.filter(
    (line) => line.trim().length > 0 && !TRUNCATION_FOOTER.test(line),
  );
}

function notableCandidates(lines: readonly string[]): string[] {
  return lines.filter(isNotable);
}

function isNotable(line: string): boolean {
  return (
    FAILURE.test(line) ||
    STACK_FRAME.test(line) ||
    PATH_LINE.test(line) ||
    SIGNATURE.test(line) ||
    EXPORT.test(line) ||
    TEST_SUMMARY.test(line) ||
    DIFF_HEADER.test(line)
  );
}

/** Trimmed, collapsed, clipped, deduplicated; `lossless` when nothing was cut or merged. */
function normalize(lines: readonly string[]): {
  lines: string[];
  lossless: boolean;
} {
  const seen = new Set<string>();
  const out: string[] = [];
  let lossless = true;
  for (const line of lines) {
    const collapsed = line.replace(/\s+/g, " ").trim();
    const text = clipEnd(collapsed, CANDIDATE_CHAR_LIMIT);
    if (text !== collapsed) lossless = false;
    if (text.length === 0) continue;
    if (seen.has(text)) {
      lossless = false;
      continue;
    }
    seen.add(text);
    out.push(text);
  }
  return { lines: out, lossless };
}

function capCandidates(lines: string[]): string[] {
  if (lines.length <= MAX_CANDIDATES) return lines;
  const tail = MAX_CANDIDATES - HEAD_CANDIDATES;
  return [
    ...lines.slice(0, HEAD_CANDIDATES),
    ...lines.slice(lines.length - tail),
  ];
}
