/**
 * The Noul questions the jev scorer asks about one chunk, and the reader
 * for their answers.
 *
 * Guarantees: every question is a Noul whose `true` and `false` criteria
 * draw the boundary in words, refers to state by backticked paths
 * (`chunk`, `task`, `ledger`, `recent_assistant`), and asks about what the
 * output shows, never about intent, counts, or arithmetic. A `keep_<i>`
 * question quotes its line in the instructions, so Jev need not resolve
 * an index. The text is model-visible to Jev; tests pin it verbatim.
 *
 * Intentionally not in the Log: the questions and the answers. Tiers and
 * the probabilities in the reason are the record.
 */

import { isFiniteNumber, isRecord } from "@sage/core";

/** One Noul question as the System One API takes it. */
export type NoulQuestion = {
  type: "noul";
  instructions: string | { line: string; question: string };
  criteria: { true: string; false: string };
};

/** The four judgments asked of every chunk, by question id. */
const FIXED_QUESTIONS = {
  reports_failure: noul(
    "Does the tool output in `chunk` report that something failed?",
    "The output reports a failure: an error message, an exception or stack trace, a non-zero exit code, a failing test or assertion, a failed build, lint, or type check, or `chunk.is_error` is true.",
    "The output reports success or only shows data such as file contents, a listing, or search hits. Error words that appear inside file contents or source code being read do not count.",
  ),
  is_edit_target: noul(
    "Is the file in `chunk` a file that `recent_assistant` or `ledger` says will be edited?",
    "`chunk` shows one file, and `recent_assistant` or `ledger` says that file will be changed, fixed, or written.",
    "No edit to this file is mentioned, the file is read only for reference, or `chunk` is a listing, a search across files, or command output rather than one file.",
  ),
  answers_lookup: noul(
    "Does the tool output in `chunk` contain the specific symbol, line, or value that `recent_assistant` says it is looking for?",
    "`recent_assistant` names a specific thing to find, such as a definition, a call site, a setting, or an error line, and the output in `chunk` shows it.",
    "`recent_assistant` does not name a specific thing to find, or the output in `chunk` does not show it.",
  ),
  is_broad_listing: noul(
    "Is the tool output in `chunk` only a directory listing or a broad search result?",
    "The output is a list of file or directory names, or search hits spread over many files, and no single entry is singled out by `task`, `ledger`, or `recent_assistant`.",
    "The output is file contents, command or test output, a diff, or a narrow search whose hits are the code that `task` or `recent_assistant` is about.",
  ),
} as const;

const KEEP_QUESTION =
  "If the tool output in `chunk` were replaced by `line` alone, would `line` tell a reader where the important result is without rereading the output?";
const KEEP_TRUE =
  "`line` names a concrete location or result that matters for `task`: a file path with a line number, an error or failing test, a function, class, or export signature, a test summary, or a diff hunk header.";
const KEEP_FALSE =
  "`line` is generic, boilerplate, or unrelated to `task`, or it carries no path, name, or result a reader could look up.";

/** The probability of yes for each judgment of one chunk. */
export type Judgment = {
  failure: number;
  edit: number;
  lookup: number;
  broad: number;
  /** `keep_<i>` by candidate index; a missing answer reads as 0. */
  keep: number[];
};

/** The questions for one chunk: the four fixed Nouls, then `keep_<i>` per candidate. */
export function questionsFor(
  candidates: readonly string[],
): Record<string, NoulQuestion> {
  const questions: Record<string, NoulQuestion> = { ...FIXED_QUESTIONS };
  candidates.forEach((line, index) => {
    questions[`keep_${index}`] = {
      type: "noul",
      instructions: { line, question: KEEP_QUESTION },
      criteria: { true: KEEP_TRUE, false: KEEP_FALSE },
    };
  });
  return questions;
}

/**
 * Reads the answers for one chunk. Returns undefined when any of the four
 * fixed answers is missing or not a finite `noul`, so the chunk gets no
 * decision.
 */
export function readJudgment(
  answers: Record<string, unknown>,
  candidateCount: number,
): Judgment | undefined {
  const failure = readNoul(answers.reports_failure);
  const edit = readNoul(answers.is_edit_target);
  const lookup = readNoul(answers.answers_lookup);
  const broad = readNoul(answers.is_broad_listing);
  if (
    failure === undefined ||
    edit === undefined ||
    lookup === undefined ||
    broad === undefined
  ) {
    return undefined;
  }
  const keep = Array.from(
    { length: candidateCount },
    (_, index) => readNoul(answers[`keep_${index}`]) ?? 0,
  );
  return { failure, edit, lookup, broad, keep };
}

function readNoul(answer: unknown): number | undefined {
  if (!isRecord(answer) || !isFiniteNumber(answer.noul)) return undefined;
  return answer.noul;
}

function noul(instructions: string, yes: string, no: string): NoulQuestion {
  return { type: "noul", instructions, criteria: { true: yes, false: no } };
}
