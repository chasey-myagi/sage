/**
 * argv parsing for the one-shot `sage [-p] <task>` command.
 *
 * Contract: returns either the parsed options or the first error in argv
 * order. The task is the value after `-p`/`--print` (or `--print=task`), or
 * else the positionals joined by spaces; both at once is a usage error.
 * `--help` returns the usage text as an error with exit 1. Flag semantics are
 * documented in `apps/sage/README.md`.
 *
 * Intentionally not in the Log: argv and the parsed options.
 */

import type { ToolNetwork } from "@sage/tools";
import { benchRunUsage } from "./bench/args.js";
import { assertNever } from "./guards.js";
import {
  type CommandFlags,
  checkChoice,
  checkPositiveInt,
  checkToolNetwork,
  type FlagError,
  type LadderRung,
  type OptionName,
  parseLadder,
  readFlags,
  renderUsage,
  SCORER_APPLY_MODES,
  SCORER_NAMES,
  type ScorerApply,
  type ScorerName,
  usageError,
} from "./options.js";

const SAGE_FLAGS = [
  { name: "print", usage: "required" },
  { name: "provider", usage: "optional" },
  { name: "model", usage: "optional" },
  { name: "base-url", usage: "optional" },
  { name: "api-key-env", usage: "optional" },
  { name: "ladder", usage: "optional" },
  { name: "budget", usage: "optional" },
  { name: "max-turns", usage: "optional" },
  { name: "tool-network", usage: "optional" },
  { name: "tool-sandbox", usage: "optional" },
  { name: "passthrough", usage: "optional" },
  { name: "scorer", usage: "optional" },
  { name: "shadow-scorer", usage: "optional" },
  { name: "scorer-apply", usage: "optional" },
  { name: "help", usage: "hidden" },
] as const satisfies CommandFlags<OptionName>;

type SageFlag = (typeof SAGE_FLAGS)[number]["name"];

/** Options for one `sage -p` run. Absent fields take the defaults. */
export type PrintOptions = {
  task: string;
  provider?: string;
  model?: string;
  baseUrl?: string;
  apiKeyEnv?: string;
  passthrough: boolean;
  scorer?: ScorerName;
  shadowScorer?: ScorerName;
  scorerApply?: ScorerApply;
  ladder?: LadderRung[];
  budget?: number;
  maxTurns?: number;
  /** Last of `--tool-network` / `--tool-sandbox`; absent means `full`. */
  toolNetwork?: ToolNetwork;
};

/** Result of {@link parseArgs}. */
export type Parsed = ({ ok: true } & PrintOptions) | FlagError;

type Draft = Omit<PrintOptions, "task"> & {
  task?: string;
  positional: string[];
};

/**
 * Parses `sage -p` argv. Never throws; the error's `message` is one stderr
 * line (or the usage block) and `exitCode` is what the process returns.
 */
export function parseArgs(argv: string[]): Parsed {
  const tokens = readFlags(argv, SAGE_FLAGS);
  const draft: Draft = { passthrough: false, positional: [] };
  for (let at = 0; at < tokens.length; at += 1) {
    const token = tokens[at];
    if (!token) continue;
    if (token.kind === "invalid") return usageError(usage());
    if (token.kind === "positional") {
      draft.positional.push(token.value);
      continue;
    }
    if (token.kind === "switch") {
      if (token.name === "help") return usageError(usage());
      if (token.name === "print") {
        // `-p <task>` takes the next arg only when it is a plain word;
        // `-p --flag task` leaves the task to the positionals.
        const next = tokens[at + 1];
        if (next?.kind === "positional" && next.index === token.index + 1) {
          if (next.value.length > 0) {
            draft.task = next.value;
            at += 1;
          }
        }
        continue;
      }
      if (token.name === "passthrough") draft.passthrough = true;
      continue;
    }
    const error = applyValue(draft, token.name, token.value);
    if (error) return error;
  }
  return finish(draft);
}

/** Usage for every subcommand; `--help` and usage errors print it. */
export function usage(): string {
  return [
    renderUsage("usage: sage", SAGE_FLAGS),
    "       sage stats <session.jsonl>",
    `       ${benchRunUsage()}`,
    "       sage bench report <runA> <runB>",
  ].join("\n");
}

function applyValue(
  draft: Draft,
  name: SageFlag,
  value: string,
): FlagError | undefined {
  switch (name) {
    case "print":
      draft.task = value;
      return undefined;
    case "provider":
      draft.provider = value;
      return undefined;
    case "model":
      draft.model = value;
      return undefined;
    case "base-url":
      draft.baseUrl = value;
      return undefined;
    case "api-key-env":
      draft.apiKeyEnv = value;
      return undefined;
    case "ladder": {
      const ladder = parseLadder(value);
      if (!ladder) return usageError(usage());
      draft.ladder = ladder;
      return undefined;
    }
    case "budget":
    case "max-turns":
      return applyNumber(draft, name, value);
    case "tool-network":
    case "tool-sandbox": {
      const network = checkToolNetwork(name, value);
      if (!network.ok) return network;
      draft.toolNetwork = network.value;
      return undefined;
    }
    case "scorer":
    case "shadow-scorer":
    case "scorer-apply":
      return applyScoring(draft, name, value);
    case "passthrough":
    case "help":
      // readFlags never yields a value for these; they take none.
      return usageError(usage());
    default:
      return assertNever(name);
  }
}

function applyNumber(
  draft: Draft,
  name: "budget" | "max-turns",
  value: string,
): FlagError | undefined {
  const checked = checkPositiveInt(name, value);
  if (!checked.ok) return checked;
  if (name === "budget") draft.budget = checked.value;
  else draft.maxTurns = checked.value;
  return undefined;
}

function applyScoring(
  draft: Draft,
  name: "scorer" | "shadow-scorer" | "scorer-apply",
  value: string,
): FlagError | undefined {
  if (name === "scorer-apply") {
    const mode = checkChoice(SCORER_APPLY_MODES, value, usage());
    if (!mode.ok) return mode;
    draft.scorerApply = mode.value;
    return undefined;
  }
  const scorer = checkChoice(SCORER_NAMES, value, usage());
  if (!scorer.ok) return scorer;
  if (name === "scorer") draft.scorer = scorer.value;
  else draft.shadowScorer = scorer.value;
  return undefined;
}

function finish({ positional, task, ...options }: Draft): Parsed {
  if (task !== undefined && positional.length > 0) {
    return usageError(usage());
  }
  const resolved = task ?? positional.join(" ");
  if (!resolved) return usageError(usage());
  return { ok: true, task: resolved, ...options };
}
