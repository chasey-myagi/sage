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
import type { Chunk } from "../compiler/chunks.js";
import { classifyBashCommand } from "./bash-kind.js";
import type { ScoreDecision, Scorer } from "./types.js";

/**
 * What the rule scorer judges a candidate by. `echo`: the result repeats
 * what the call already says (an edit, a write, a ledger update). `reader`:
 * `read` or `grep` output. `bash-*`: a `bash` call by `classifyBashCommand`,
 * with a failed runner split out as `bash-runner-error`. `other`: any other
 * tool, or a `bash` call without a string `command`.
 */
export type RuleCategory =
  | "echo"
  | "reader"
  | "bash-reader"
  | "bash-runner"
  | "bash-runner-error"
  | "bash-mixed"
  | "other";

/** A category's tier; `keep` gives no decision, so the compiler's own rules stand. */
export type RuleTier = "none" | "summary" | "full" | "keep";

/**
 * Default tier per category. Echoes are rarely needed again, since the
 * call's own input already holds the change, so they drop to the stub.
 * Reads, searches, and read-only bash are often cited later and keep. A
 * runner's output (tests, builds, scripts) is rarely needed once it
 * succeeded, so it becomes a stub-only summary; a failed one keeps, since
 * the error is what the next step works from. Mixed commands and other
 * tools keep.
 */
export const DEFAULT_RULE_TIERS: Readonly<Record<RuleCategory, RuleTier>> = {
  echo: "none",
  reader: "keep",
  "bash-reader": "keep",
  "bash-runner": "summary",
  "bash-runner-error": "keep",
  "bash-mixed": "keep",
  other: "keep",
};

/**
 * Category by tool name for tools judged by name alone. The names match
 * `TOOL_NAMES` in `@sage/tools` and the `progressLedger` tool, spelled here
 * because plugins do not import the tools package.
 */
const RULE_TOOL_CATEGORIES: ReadonlyMap<string, RuleCategory> = new Map<
  string,
  RuleCategory
>([
  ["edit", "echo"],
  ["write", "echo"],
  ["update_ledger", "echo"],
  ["read", "reader"],
  ["grep", "reader"],
]);

/** The tool whose calls `classifyBashCommand` judges. */
const BASH_TOOL_NAME = "bash";

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

/** The category `ruleScorer` judges `chunk` by. */
function ruleCategory(chunk: Chunk): RuleCategory {
  const name = chunk.toolName ?? "";
  if (name !== BASH_TOOL_NAME) return RULE_TOOL_CATEGORIES.get(name) ?? "other";
  const command = isRecord(chunk.input) ? chunk.input.command : undefined;
  if (typeof command !== "string") return "other";
  const kind = classifyBashCommand(command);
  if (kind === "runner" && chunk.isError === true) return "bash-runner-error";
  return `bash-${kind}`;
}
