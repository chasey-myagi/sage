/**
 * Deterministic scorer: tiers each candidate by the tool that produced it,
 * with no request.
 *
 * Contract: `name` is `rule` and `isLocal` is true. `score` resolves with
 * `requested: false`, no `usage`, and no `fallback`, and reads neither
 * `prevRequest` nor `query`. A candidate gets a decision only when its
 * category's tier is not `keep`; the decision's reason is
 * `scorer:rule:<category>`. A `summary` decision is `stubOnly`, so the
 * view shows the stub line and no generated text.
 *
 * Intentionally not in the Log: the category of an undecided candidate and
 * the tier table. A decided chunk's reason names its category.
 */

import { isRecord } from "@sage/core";
import { LEDGER_TOOL_NAME } from "../claims/progress-ledger.js";
import type { Chunk } from "../compiler/chunks.js";
import { classifyBashCommand } from "./bash-kind.js";
import type { ScoreDecision, Scorer } from "./types.js";

/**
 * What the rule scorer judges a candidate by. `echo`: the result repeats
 * what the call already says (an edit, a write, a ledger update). `reader`:
 * `read` or `grep` output. `bash-*`: a `bash` call by `classifyBashCommand`,
 * with a runner whose result is `isError` split out as `bash-runner-error`.
 * `other`: any other tool, or a `bash` call without a string `command`.
 */
export type RuleCategory =
  | "echo"
  | "reader"
  | "bash-reader"
  | "bash-runner"
  | "bash-runner-error"
  | "bash-mixed"
  | "other";

/**
 * A category's tier; `keep` gives no decision, so the compiler's own rules
 * stand.
 */
export type RuleTier = "none" | "summary" | "full" | "keep";

/**
 * Default tier per category. Only `bash-runner` changes a tier: a test,
 * build, or script that exited 0 is rarely needed again, so it becomes a
 * stub-only summary. Everything else keeps:
 * - `echo`: the real results are one line (`Edited <path>`, `Wrote <path>`,
 *   `Ledger updated.`). The stub prints the call's input and token count,
 *   so it would be longer than the result and suggest omitted content.
 * - `reader`, `bash-reader`: file content and search hits are often cited
 *   later.
 * - `bash-runner-error`: the error is what the next step works from.
 * - `bash-mixed`: covers runners piped onward without `pipefail`, whose
 *   `isError` cannot reflect the runner's own failure, and commands whose
 *   output may hold what was read.
 * - `other`: no basis for a decision.
 */
export const DEFAULT_RULE_TIERS: Readonly<Record<RuleCategory, RuleTier>> = {
  echo: "keep",
  reader: "keep",
  "bash-reader": "keep",
  "bash-runner": "summary",
  "bash-runner-error": "keep",
  "bash-mixed": "keep",
  other: "keep",
};

/**
 * Category by tool name for tools judged by name alone. `edit`, `write`,
 * `read`, and `grep` match `TOOL_NAMES` in `@sage/tools`, spelled here
 * because plugins do not import the tools package; the ledger name comes
 * from `progressLedger`.
 */
const RULE_TOOL_CATEGORIES: ReadonlyMap<string, RuleCategory> = new Map<
  string,
  RuleCategory
>([
  ["edit", "echo"],
  ["write", "echo"],
  [LEDGER_TOOL_NAME, "echo"],
  ["read", "reader"],
  ["grep", "reader"],
]);

/**
 * The tool whose calls `classifyBashCommand` judges. Not named
 * `*_TOOL_NAME`: check-layers collects those as plugin tool names.
 */
const BASH_TOOL = "bash";

/** Prefix of the reason a rule decision records. */
const RULE_REASON_PREFIX = "scorer:rule:";

/** Options of `ruleScorer`. */
type RuleScorerOptions = {
  /** Per-category overrides of `DEFAULT_RULE_TIERS`. */
  tiers?: Partial<Record<RuleCategory, RuleTier>>;
};

/** Creates the rule scorer. It never throws and never sends a request. */
export function ruleScorer(opts: RuleScorerOptions = {}): Scorer {
  const tiers = { ...DEFAULT_RULE_TIERS, ...opts.tiers };
  return {
    name: "rule",
    isLocal: true,
    score(input) {
      const decisions: ScoreDecision[] = [];
      for (const chunk of input.candidates) {
        const category = ruleCategory(chunk);
        const tier = tiers[category];
        if (tier === "keep") continue;
        decisions.push({
          id: chunk.id,
          tier,
          reason: `${RULE_REASON_PREFIX}${category}`,
          ...(tier === "summary" ? { stubOnly: true as const } : {}),
        });
      }
      return Promise.resolve({ decisions, requested: false });
    },
  };
}

/**
 * The category `ruleScorer` judges a tool chunk by, from its tool name,
 * input, and `isError`. Callers outside the scorer reuse it so the tool
 * name table has one definition. A prose chunk (no `toolName`) is `other`.
 */
export function ruleCategory(
  chunk: Pick<Chunk, "toolName" | "input" | "isError">,
): RuleCategory {
  const name = chunk.toolName ?? "";
  if (name !== BASH_TOOL) return RULE_TOOL_CATEGORIES.get(name) ?? "other";
  const command = isRecord(chunk.input) ? chunk.input.command : undefined;
  if (typeof command !== "string") return "other";
  const kind = classifyBashCommand(command);
  if (kind === "runner" && chunk.isError === true) return "bash-runner-error";
  return `bash-${kind}`;
}
