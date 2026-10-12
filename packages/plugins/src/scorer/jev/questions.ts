/**
 * The Noul questions the jev scorer asks about one chunk, which chunks
 * are asked at all, and the reader for their answers.
 *
 * Guarantees: every question is a Noul whose `true` and `false` criteria
 * draw the boundary in words, refers to state by backticked paths
 * (`chunk`, `task`, `ledger`, `recent_assistant`), and asks about what the
 * output shows, never about intent, counts, or arithmetic. Each Noul
 * judges one condition. A `keep_<i>` statement quotes its line in the
 * instructions, so Jev need not resolve an index. The text is
 * model-visible to Jev; tests pin it verbatim. Only grep and runner or
 * mixed bash get questions that decide a tier; read gets one probe
 * question; every other chunk gets none. The chunk kind comes from the
 * rule scorer's `ruleCategory`, so tool names and bash classification
 * have one definition.
 *
 * Intentionally not in the Log: the questions and the answers. Tiers and
 * the probabilities in the reason are the record.
 */

import { assertNever, isFiniteNumber, isRecord } from "@sage/core";
import type { Chunk } from "../../compiler/chunks.js";
import type { BashKind } from "../bash-kind.js";
import { READ_TOOL, type RuleCategory, ruleCategory } from "../rule.js";

/** One Noul question as the System One API takes it. */
export type NoulQuestion = {
  type: "noul";
  instructions: string | { line: string; statement: string };
  criteria: { true: string; false: string };
};

/**
 * Which questions a chunk is asked. `triage` (grep, runner or mixed
 * bash): `reports_failure`, `locates_next_step`, and `keep_<i>` per
 * candidate; its answers decide the tier. `read-probe` (read): only
 * `shows_target_code`, an experiment that decides nothing and is asked
 * only by offline calibration.
 */
export type QuestionSet = "triage" | "read-probe";

/** The fields of a chunk its question set and bash kind are judged from. */
type ChunkKindFields = Pick<Chunk, "toolName" | "input" | "isError">;

/** `classifyBashCommand`'s verdict behind each bash category. */
const BASH_KINDS: Readonly<Partial<Record<RuleCategory, BashKind>>> = {
  "bash-reader": "reader",
  "bash-runner": "runner",
  "bash-runner-error": "runner",
  "bash-mixed": "mixed",
};

const TRIAGE_QUESTIONS = {
  reports_failure: noul(
    "Does the tool output in `chunk` report that something failed?",
    "The output reports an error, an exception, a non-zero exit code, a failing test, or a failed build or check.",
    "The output reports success or only shows data; error words inside file contents being read do not count.",
  ),
  locates_next_step: noul(
    "Does the tool output in `chunk` name a file path, line number, test name, or error location that `recent_assistant` and `ledger` have not already mentioned?",
    "The output shows at least one path, line, test name, or error location that does not appear in `recent_assistant` or `ledger`.",
    "The output only confirms success, repeats locations already named, or shows no location at all.",
  ),
} as const;

const READ_PROBE_QUESTIONS = {
  shows_target_code: noul(
    "Does `chunk` show the body of the function, class, or block that `task` or `ledger` names as the thing to change or verify?",
    "The excerpt contains the definition or body lines of a symbol that `task` or `ledger` names.",
    "The excerpt shows other parts of the file, such as imports, headers, or unrelated helpers, or `task` and `ledger` name no symbol.",
  ),
} as const;

/**
 * The `keep_<i>` statement, also its `true` criterion: one direct claim
 * about the quoted line, no counterfactual. Criteria stay under 80
 * characters so the repeated boilerplate is about 270 bytes per line.
 */
const KEEP_STATEMENT =
  "`line` names a location or result `task` or `recent_assistant` needs.";
const KEEP_FALSE = "`line` is boilerplate or unrelated to `task`.";

/** The probability of yes for each triage judgment of one chunk. */
export type TriageJudgment = {
  set: "triage";
  failure: number;
  locate: number;
  /** `keep_<i>` by candidate index; a missing answer reads as 0. */
  keep: number[];
};

/** The probability of yes for the read probe of one chunk. */
export type ReadProbeJudgment = {
  set: "read-probe";
  target: number;
};

/** One chunk's answers, by the question set it was asked. */
export type Judgment = TriageJudgment | ReadProbeJudgment;

/**
 * The question set for `chunk`, or undefined when it is not asked: by its
 * `ruleCategory`, read is a `read-probe`; grep and runner or mixed bash,
 * `isError` included, are `triage`; reader bash, bash without a string
 * `command`, and every other tool are not asked.
 */
export function questionSetFor(
  chunk: ChunkKindFields,
): QuestionSet | undefined {
  const category = ruleCategory(chunk);
  switch (category) {
    case "reader":
      return chunk.toolName === READ_TOOL ? "read-probe" : "triage";
    case "bash-runner":
    case "bash-runner-error":
    case "bash-mixed":
      return "triage";
    case "bash-reader":
    case "echo":
    case "other":
      return undefined;
    default:
      return assertNever(category);
  }
}

/** `classifyBashCommand`'s verdict on a bash `chunk`; null for any other chunk. */
export function bashKindOf(chunk: ChunkKindFields): BashKind | null {
  return BASH_KINDS[ruleCategory(chunk)] ?? null;
}

/**
 * The questions for one chunk: for `triage`, the two fixed Nouls then
 * `keep_<i>` per candidate; for `read-probe`, `shows_target_code` alone.
 */
export function questionsFor(
  set: QuestionSet,
  candidates: readonly string[],
): Record<string, NoulQuestion> {
  if (set === "read-probe") return { ...READ_PROBE_QUESTIONS };
  const questions: Record<string, NoulQuestion> = { ...TRIAGE_QUESTIONS };
  candidates.forEach((line, index) => {
    questions[`keep_${index}`] = {
      type: "noul",
      instructions: { line, statement: KEEP_STATEMENT },
      criteria: { true: KEEP_STATEMENT, false: KEEP_FALSE },
    };
  });
  return questions;
}

/**
 * Reads the answers for one chunk asked `set`. Returns undefined when a
 * fixed answer of that set is missing or not a finite `noul`, so the
 * chunk gets no decision.
 */
export function readJudgment(
  answers: Record<string, unknown>,
  set: QuestionSet,
  candidateCount: number,
): Judgment | undefined {
  if (set === "read-probe") {
    const target = readNoul(answers.shows_target_code);
    return target === undefined ? undefined : { set, target };
  }
  const failure = readNoul(answers.reports_failure);
  const locate = readNoul(answers.locates_next_step);
  if (failure === undefined || locate === undefined) return undefined;
  const keep = Array.from(
    { length: candidateCount },
    (_, index) => readNoul(answers[`keep_${index}`]) ?? 0,
  );
  return { set, failure, locate, keep };
}

function readNoul(answer: unknown): number | undefined {
  if (!isRecord(answer) || !isFiniteNumber(answer.noul)) return undefined;
  return answer.noul;
}

function noul(instructions: string, yes: string, no: string): NoulQuestion {
  return { type: "noul", instructions, criteria: { true: yes, false: no } };
}
