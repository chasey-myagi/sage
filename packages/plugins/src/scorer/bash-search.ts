/**
 * Bash command text read for offline labels: a comparison key that tells
 * whether two commands are the same, and the patterns a command's grep
 * and rg stages search for.
 *
 * Contract: every export is a pure function of the command text and uses
 * the quote-aware split of `bash-kind.ts`; nothing here parses a second
 * way.
 *
 * Intentionally not in the Log: the keys and patterns. Callers derive
 * them from tool inputs the Log already holds.
 */

import { commandWords, splitParts, withoutHeredocBodies } from "./bash-kind.js";

/** Search commands whose pattern {@link bashSearchPatterns} reads. */
const BASH_SEARCH_COMMANDS: ReadonlySet<string> = new Set([
  "grep",
  "egrep",
  "fgrep",
  "rg",
]);

/**
 * grep and rg options that take the next word as their value, so that
 * word is not the pattern. Long options written `--name=value` are one
 * word and need no entry.
 */
const SEARCH_OPTIONS_WITH_VALUE: ReadonlySet<string> = new Set([
  "-A",
  "-B",
  "-C",
  "-m",
  "-g",
  "-t",
  "-T",
  "-d",
  "-D",
  "-j",
  "-M",
  "--glob",
  "--type",
  "--type-not",
  "--max-count",
  "--context",
  "--after-context",
  "--before-context",
  "--include",
  "--exclude",
  "--exclude-dir",
  "--max-depth",
]);

/** `--regexp=<pattern>` written as one word. */
const REGEXP_INLINE = "--regexp=";

/**
 * A comparison key for a bash command: two commands get the same key when
 * the quote-aware scan of `classifyBashCommand` splits them into the
 * same words, stages, and parts. Whitespace runs, quoting that does not
 * change a word, comments, and empty parts do not change the key. Unlike
 * classification, heredoc bodies are kept, so two writes of different
 * text stay different. The key is not a runnable command.
 */
export function normalizeBashCommand(command: string): string {
  const parts = splitParts(command)
    .map((stages) =>
      stages
        .filter((words) => words.length > 0)
        .map((words) => words.map((word) => word.text)),
    )
    .filter((stages) => stages.length > 0);
  return JSON.stringify(parts);
}

/**
 * The search patterns of every `grep`, `egrep`, `fgrep`, or `rg` stage
 * in a bash command, in order: each `-e` / `--regexp` value, or else the
 * first word that is not an option. Stages are found as
 * `classifyBashCommand` finds them, wrappers included (`xargs grep
 * x`); heredoc bodies are dropped. A stage whose pattern cannot be told
 * (none given, or read from a file with `-f`) adds nothing.
 */
export function bashSearchPatterns(command: string): string[] {
  const patterns: string[] = [];
  for (const stages of splitParts(withoutHeredocBodies(command))) {
    for (const stage of stages) {
      const words = commandWords(stage);
      if (words === "compound") continue;
      const name = words[0]?.text;
      if (name === undefined || !BASH_SEARCH_COMMANDS.has(name)) continue;
      patterns.push(...searchPatterns(words.slice(1).map((w) => w.text)));
    }
  }
  return patterns;
}

/** Patterns given to one search stage, from its words after the name. */
function searchPatterns(args: readonly string[]): string[] {
  const explicit: string[] = [];
  for (let index = 0; index < args.length; index += 1) {
    const word = args[index] ?? "";
    if (word === "--") {
      const next = args[index + 1];
      return explicit.length > 0 || next === undefined ? explicit : [next];
    }
    if (word === "-e" || word === "--regexp") {
      const value = args[index + 1];
      if (value !== undefined) explicit.push(value);
      index += 1;
      continue;
    }
    if (word.startsWith(REGEXP_INLINE)) {
      explicit.push(word.slice(REGEXP_INLINE.length));
      continue;
    }
    if (word === "-f" || word.startsWith("--file")) return explicit;
    if (word.startsWith("-")) {
      if (SEARCH_OPTIONS_WITH_VALUE.has(word)) index += 1;
      continue;
    }
    return explicit.length > 0 ? explicit : [word];
  }
  return explicit;
}
