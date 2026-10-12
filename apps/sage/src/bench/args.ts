/**
 * argv parsing for `sage bench run` and `sage bench report`.
 *
 * Contract: every flag takes a value; positionals and unknown flags print
 * the bench usage and exit 1. Repeated flags keep the last value, and
 * `--tool-sandbox` is the same switch as `--tool-network`. Values are
 * checked after the whole argv is read, in a fixed order, with the same
 * checks and messages as `sage -p`. `bench report` takes exactly two run
 * directories and the `--exclude-faults` switch, in any order. Flag
 * semantics are in `apps/sage/README.md`.
 *
 * Intentionally not in the Log: argv and the parsed options.
 */

import { DEFAULT_MAX_TURNS } from "@sage/core";
import { resolvePreset } from "@sage/provider";
import type { ToolNetwork } from "@sage/tools";
import {
  BENCH_POLICIES,
  type BenchPolicy,
  type Checked,
  type CommandFlags,
  checkChoice,
  checkPositiveInt,
  checkToolNetwork,
  type OptionName,
  readFlags,
  renderUsage,
  SCORER_APPLY_MODES,
  SCORER_NAMES,
  type ScorerApply,
  type ScorerName,
  usageError,
} from "../options.js";

/** Flags `sage bench run` accepts, in usage order. */
const BENCH_RUN_FLAGS = [
  { name: "tasks", usage: "required" },
  { name: "policy", usage: "required" },
  { name: "model", usage: "required" },
  { name: "provider", usage: "optional" },
  { name: "base-url", usage: "optional" },
  { name: "api-key-env", usage: "optional" },
  { name: "budget", usage: "optional" },
  { name: "max-turns", usage: "optional" },
  { name: "tool-network", usage: "optional" },
  { name: "tool-sandbox", usage: "optional" },
  { name: "scorer", usage: "optional" },
  { name: "shadow-scorer", usage: "optional" },
  { name: "scorer-apply", usage: "optional" },
  { name: "out", usage: "required" },
] as const satisfies CommandFlags<OptionName>;

/** Flags `sage bench report` accepts; the two run directories are positionals. */
const BENCH_REPORT_FLAGS = [
  { name: "exclude-faults", usage: "optional" },
] as const satisfies CommandFlags<OptionName>;

type BenchFlag = (typeof BENCH_RUN_FLAGS)[number]["name"];
type NetworkFlag = "tool-network" | "tool-sandbox";

/** Parsed `sage bench run` options with defaults filled in. */
export type BenchRunArgs = {
  tasks: string;
  policy: BenchPolicy;
  model: string;
  provider?: string;
  /** `--base-url`, forwarded to each child when written. */
  baseUrl?: string;
  /** `--api-key-env`, forwarded to each child when written. */
  apiKeyEnv?: string;
  out: string;
  /** `--budget`, or null when omitted. */
  budget: number | null;
  /** Defaults to `none` for bench. */
  toolNetwork: ToolNetwork;
  /** `--max-turns`, or the kernel default when omitted. */
  maxTurns: number;
  shadowScorer?: ScorerName;
  scorer?: ScorerName;
  /** Recorded in results.json; `rebuild` when omitted. */
  scorerApply: ScorerApply;
  /** True when `--scorer-apply` was written, so it is forwarded to sage. */
  forwardScorerApply: boolean;
};

/** The `sage bench run` usage line, without the `usage:` prefix. */
export function benchRunUsage(): string {
  return renderUsage("sage bench run", BENCH_RUN_FLAGS);
}

/** Parsed `sage bench report` arguments. */
export type BenchReportArgs = {
  left: string;
  right: string;
  /** `--exclude-faults`: drop every task either run faulted on from both. */
  excludeFaults: boolean;
};

/** The `sage bench report` usage line, without the `usage:` prefix. */
export function benchReportUsage(): string {
  return `${renderUsage("sage bench report", BENCH_REPORT_FLAGS)} <runA> <runB>`;
}

/** Usage for both bench subcommands. */
export function benchUsage(): string {
  return [`usage: ${benchRunUsage()}`, `       ${benchReportUsage()}`].join(
    "\n",
  );
}

/** Parses `sage bench report` argv; undefined means a usage error. */
export function parseReport(
  argv: readonly string[],
): BenchReportArgs | undefined {
  const dirs: string[] = [];
  let excludeFaults = false;
  for (const token of readFlags(argv, BENCH_REPORT_FLAGS)) {
    if (token.kind === "switch" && token.name === "exclude-faults") {
      excludeFaults = true;
      continue;
    }
    if (token.kind !== "positional") return undefined;
    dirs.push(token.value);
  }
  const [left, right] = dirs;
  if (dirs.length !== 2 || !left || !right) return undefined;
  return { left, right, excludeFaults };
}

/** Parses `sage bench run` argv. Never throws. */
export function parseRun(argv: string[]): Checked<BenchRunArgs> {
  const collected = collectValues(argv);
  if (!collected) return usageError(benchUsage());
  const { values, networkFlag } = collected;
  const tasks = values.get("tasks");
  const rawPolicy = values.get("policy");
  const model = values.get("model");
  const out = values.get("out");
  if (!tasks || !rawPolicy || !model || !out) return usageError(benchUsage());
  const policy = checkChoice(BENCH_POLICIES, rawPolicy, benchUsage());
  if (!policy.ok) return policy;
  const provider = values.get("provider");
  if (provider && !resolvePreset(provider)) return usageError(benchUsage());
  const baseUrl = values.get("base-url");
  const apiKeyEnv = values.get("api-key-env");
  const limits = readLimits(values);
  if (!limits.ok) return limits;
  const scoring = readScoring(values);
  if (!scoring.ok) return scoring;
  const rawNetwork = values.get("tool-network");
  let toolNetwork: ToolNetwork = "none";
  if (rawNetwork !== undefined) {
    const network = checkToolNetwork(networkFlag, rawNetwork);
    if (!network.ok) return network;
    toolNetwork = network.value;
  }
  return {
    ok: true,
    value: {
      tasks,
      policy: policy.value,
      model,
      out,
      toolNetwork,
      ...limits.value,
      ...scoring.value,
      ...(provider ? { provider } : {}),
      ...(baseUrl ? { baseUrl } : {}),
      ...(apiKeyEnv ? { apiKeyEnv } : {}),
    },
  };
}

/**
 * Reads argv into a last-wins map. `--tool-sandbox` is stored as
 * `tool-network`; `networkFlag` remembers which spelling set it last so an
 * invalid value names the flag the user wrote. Undefined means usage error.
 */
function collectValues(
  argv: string[],
): { values: Map<BenchFlag, string>; networkFlag: NetworkFlag } | undefined {
  const values = new Map<BenchFlag, string>();
  let networkFlag: NetworkFlag = "tool-network";
  for (const token of readFlags(argv, BENCH_RUN_FLAGS)) {
    if (token.kind !== "value") return undefined;
    if (token.name === "tool-network" || token.name === "tool-sandbox") {
      networkFlag = token.name;
      values.set("tool-network", token.value);
      continue;
    }
    values.set(token.name, token.value);
  }
  return { values, networkFlag };
}

function readLimits(
  values: Map<BenchFlag, string>,
): Checked<Pick<BenchRunArgs, "budget" | "maxTurns">> {
  const rawBudget = values.get("budget");
  let budget: number | null = null;
  if (rawBudget !== undefined) {
    const checked = checkPositiveInt("budget", rawBudget);
    if (!checked.ok) return checked;
    budget = checked.value;
  }
  const rawMaxTurns = values.get("max-turns");
  let maxTurns = DEFAULT_MAX_TURNS;
  if (rawMaxTurns !== undefined) {
    const checked = checkPositiveInt("max-turns", rawMaxTurns);
    if (!checked.ok) return checked;
    maxTurns = checked.value;
  }
  return { ok: true, value: { budget, maxTurns } };
}

function readScoring(
  values: Map<BenchFlag, string>,
): Checked<
  Pick<
    BenchRunArgs,
    "shadowScorer" | "scorer" | "scorerApply" | "forwardScorerApply"
  >
> {
  const shadow = optionalChoice(SCORER_NAMES, values.get("shadow-scorer"));
  if (shadow && !shadow.ok) return shadow;
  const scorer = optionalChoice(SCORER_NAMES, values.get("scorer"));
  if (scorer && !scorer.ok) return scorer;
  const apply = optionalChoice(SCORER_APPLY_MODES, values.get("scorer-apply"));
  if (apply && !apply.ok) return apply;
  return {
    ok: true,
    value: {
      ...(shadow ? { shadowScorer: shadow.value } : {}),
      ...(scorer ? { scorer: scorer.value } : {}),
      scorerApply: apply ? apply.value : "rebuild",
      forwardScorerApply: apply !== undefined,
    },
  };
}

function optionalChoice<T extends string>(
  choices: readonly T[],
  raw: string | undefined,
): Checked<T> | undefined {
  if (raw === undefined) return undefined;
  return checkChoice(choices, raw, benchUsage());
}
