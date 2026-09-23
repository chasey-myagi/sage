/**
 * The one option table shared by `sage -p` and `sage bench run`, the argv
 * tokenizer built on `node:util` parseArgs, and the value checks both
 * commands apply.
 *
 * Contract: each command names the options it accepts; anything else is a
 * usage error. Tokens come back in argv order so each command can report the
 * first error it meets. Invalid numbers and `--tool-network` values exit 2
 * with a one-line message; other bad values print the command's usage and
 * exit 1. Usage lines are rendered from the table, so a flag's hint is
 * written once.
 *
 * Intentionally not in the Log: argv and every parsed value. The session
 * sees only what the commands pass to `createCodingSession`.
 */

import { parseArgs as tokenizeArgv } from "node:util";
import type { ToolNetwork } from "@sage/tools";

/** Scorers a compiler run can install: rule, model, or TypeSafe Jev. */
export const SCORER_NAMES = ["rule", "model", "jev"] as const;
/** One of {@link SCORER_NAMES}. */
export type ScorerName = (typeof SCORER_NAMES)[number];

/**
 * When the root compiler applies scores: `rebuild` at epoch reopen,
 * `entry` when a tool chunk first enters the view.
 */
export const SCORER_APPLY_MODES = ["entry", "rebuild"] as const;
/** One of {@link SCORER_APPLY_MODES}. */
export type ScorerApply = (typeof SCORER_APPLY_MODES)[number];

/** Bench context policies: raw passthrough or the context compiler. */
export const BENCH_POLICIES = ["passthrough", "compiler"] as const;
/** One of {@link BENCH_POLICIES}. */
export type BenchPolicy = (typeof BENCH_POLICIES)[number];

const TOOL_NETWORKS = ["none", "full"] as const satisfies ToolNetwork[];

/** One `provider/model` step of an explicit route ladder. */
export type LadderRung = { provider: string; model: string };

type OptionSpec = {
  type: "string" | "boolean";
  short?: string;
  /** Usage text for the flag, without the optional brackets. */
  hint: string;
  /** A boolean that may also carry `--name=value`, e.g. `--print=task`. */
  takesInline?: true;
};

/** Every flag either command accepts. */
export const OPTIONS = {
  print: {
    type: "boolean",
    short: "p",
    hint: "[-p | --print] <task>",
    takesInline: true,
  },
  help: { type: "boolean", short: "h", hint: "--help" },
  provider: { type: "string", hint: "--provider <preset>" },
  model: { type: "string", hint: "--model <id>" },
  "base-url": { type: "string", hint: "--base-url <url>" },
  "api-key-env": { type: "string", hint: "--api-key-env <NAME>" },
  ladder: { type: "string", hint: "--ladder provider/model,provider/model" },
  budget: { type: "string", hint: "--budget <tokens>" },
  "max-turns": { type: "string", hint: "--max-turns <n>" },
  "tool-network": { type: "string", hint: "--tool-network none|full" },
  "tool-sandbox": { type: "string", hint: "--tool-sandbox none|full" },
  passthrough: { type: "boolean", hint: "--passthrough" },
  scorer: { type: "string", hint: "--scorer rule|model|jev" },
  "shadow-scorer": { type: "string", hint: "--shadow-scorer rule|model|jev" },
  "scorer-apply": { type: "string", hint: "--scorer-apply entry|rebuild" },
  tasks: { type: "string", hint: "--tasks <file>" },
  policy: { type: "string", hint: "--policy passthrough|compiler" },
  out: { type: "string", hint: "--out <dir>" },
} as const satisfies Record<string, OptionSpec>;

/** A key of {@link OPTIONS}: a flag name without the leading dashes. */
export type OptionName = keyof typeof OPTIONS;

/** A command's flag list in usage order. `hidden` flags are accepted but not printed. */
export type CommandFlags<N extends OptionName> = readonly {
  name: N;
  usage: "required" | "optional" | "hidden";
}[];

/**
 * One argv element after tokenizing. `switch` is a boolean flag with no
 * value; `value` is a string flag, or a `takesInline` boolean written as
 * `--name=value`; `invalid` is anything the command must reject with usage.
 */
export type FlagToken<N extends OptionName> =
  | { kind: "switch"; name: N; index: number }
  | { kind: "value"; name: N; value: string; index: number }
  | { kind: "positional"; value: string; index: number }
  | { kind: "invalid"; index: number };

/** A parse failure: the stderr line and the process exit code. */
export type FlagError = { ok: false; message: string; exitCode: 1 | 2 };

/** A checked flag value, or the error the command returns as-is. */
export type Checked<T> = { ok: true; value: T } | FlagError;

/** Renders the flags after `prefix`, bracketing the optional ones. */
export function renderUsage<N extends OptionName>(
  prefix: string,
  flags: CommandFlags<N>,
): string {
  const parts = flags.flatMap(({ name, usage }) => {
    if (usage === "hidden") return [];
    const hint = OPTIONS[name].hint;
    return [usage === "required" ? hint : `[${hint}]`];
  });
  return [prefix, ...parts].join(" ");
}

/**
 * Tokenizes argv against the flags one command accepts.
 *
 * A string flag needs a non-empty value; a separate value must not start
 * with `-`, while `--flag=-5` passes the value through for the command's
 * own check. Grouped short flags (`-ph`), `-p=x`, a lone `-`, `--`, and
 * flags the command does not accept become `invalid` tokens in place.
 */
export function readFlags<N extends OptionName>(
  argv: readonly string[],
  flags: CommandFlags<N>,
): FlagToken<N>[] {
  const accepted = new Set<string>(flags.map((flag) => flag.name));
  const options = Object.fromEntries(
    flags.map(({ name }) => [name, optionConfig(name)]),
  );
  const { tokens } = tokenizeArgv({
    args: [...argv],
    options,
    strict: false,
    allowPositionals: true,
    tokens: true,
  });
  return tokens.map((token): FlagToken<N> => {
    if (token.kind === "option-terminator") {
      return { kind: "invalid", index: token.index };
    }
    if (token.kind === "positional") {
      return token.value.startsWith("-")
        ? { kind: "invalid", index: token.index }
        : { kind: "positional", value: token.value, index: token.index };
    }
    if (!accepted.has(token.name) || !isWholeArg(argv, token)) {
      return { kind: "invalid", index: token.index };
    }
    // The Set above holds exactly the names in `flags`, so the name is an N.
    return flagToken(token.name as N, token);
  });
}

type OptionToken = {
  name: string;
  rawName: string;
  index: number;
  value?: string | undefined;
  inlineValue?: boolean | undefined;
};

function optionConfig(name: OptionName): {
  type: "string" | "boolean";
  short?: string;
} {
  const spec: OptionSpec = OPTIONS[name];
  return spec.short
    ? { type: spec.type, short: spec.short }
    : { type: spec.type };
}

function isWholeArg(argv: readonly string[], token: OptionToken): boolean {
  const arg = argv[token.index];
  if (token.inlineValue) return arg === `${token.rawName}=${token.value}`;
  return arg === token.rawName;
}

function flagToken<N extends OptionName>(
  name: N,
  token: OptionToken,
): FlagToken<N> {
  const spec: OptionSpec = OPTIONS[name];
  const value = token.value;
  if (spec.type === "boolean") {
    if (!token.inlineValue) return { kind: "switch", name, index: token.index };
    if (!spec.takesInline || !value) {
      return { kind: "invalid", index: token.index };
    }
    return { kind: "value", name, value, index: token.index };
  }
  if (!value || (!token.inlineValue && value.startsWith("-"))) {
    return { kind: "invalid", index: token.index };
  }
  return { kind: "value", name, value, index: token.index };
}

/** A usage failure: the usage text on stderr, exit 1. */
export function usageError(usage: string): FlagError {
  return { ok: false, message: usage, exitCode: 1 };
}

/** Checks a `--budget` or `--max-turns` value; anything else exits 2. */
export function checkPositiveInt(
  flag: "budget" | "max-turns",
  raw: string,
): Checked<number> {
  const value = positiveInt(raw);
  if (value !== undefined) return { ok: true, value };
  return invalidValue(flag, raw, "a positive integer");
}

/**
 * Checks a `--tool-network` or `--tool-sandbox` value; anything else exits
 * 2 and names the flag as written.
 */
export function checkToolNetwork(
  flag: "tool-network" | "tool-sandbox",
  raw: string,
): Checked<ToolNetwork> {
  const value = TOOL_NETWORKS.find((network) => network === raw);
  if (value) return { ok: true, value };
  return invalidValue(flag, raw, "none or full");
}

/** Checks a value against a closed list; a miss is a usage error. */
export function checkChoice<T extends string>(
  choices: readonly T[],
  raw: string,
  usage: string,
): Checked<T> {
  const value = choices.find((choice) => choice === raw);
  if (value) return { ok: true, value };
  return usageError(usage);
}

/**
 * Parses `provider/model` rungs separated by commas. Blank rungs are
 * skipped; returns undefined when none remain or a rung lacks either side.
 */
export function parseLadder(value: string): LadderRung[] | undefined {
  const parts = value
    .split(",")
    .map((part) => part.trim())
    .filter((part) => part.length > 0);
  if (parts.length === 0) return undefined;
  const ladder: LadderRung[] = [];
  for (const part of parts) {
    const slash = part.indexOf("/");
    if (slash <= 0 || slash === part.length - 1) return undefined;
    ladder.push({
      provider: part.slice(0, slash),
      model: part.slice(slash + 1),
    });
  }
  return ladder;
}

function positiveInt(value: string): number | undefined {
  if (!/^[0-9]+$/.test(value)) return undefined;
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed <= 0) return undefined;
  return parsed;
}

function invalidValue(flag: string, raw: string, expected: string): FlagError {
  return {
    ok: false,
    exitCode: 2,
    message: `Invalid --${flag} ${JSON.stringify(raw)}: expected ${expected}`,
  };
}
