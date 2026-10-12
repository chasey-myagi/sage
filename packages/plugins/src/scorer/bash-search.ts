/**
 * Bash command text read for offline labels: a comparison key that tells
 * whether two commands are the same, the patterns a command's grep and rg
 * stages search for, and the files its reader stages open.
 *
 * Contract: every export is a pure function of the command text and uses
 * the quote-aware split and reader rules of `bash-kind.ts`; nothing here
 * parses a second way.
 *
 * Intentionally not in the Log: the keys, patterns, and operands. Callers
 * derive them from tool inputs the Log already holds.
 */

import {
  commandWords,
  isReaderStage,
  splitParts,
  type Word,
  withoutHeredocBodies,
} from "./bash-kind.js";

/** grep options that take the next word as their value. */
const GREP_OPTIONS_WITH_VALUE: ReadonlySet<string> = new Set([
  "-A",
  "-B",
  "-C",
  "-m",
  "-d",
  "-D",
  "--max-count",
  "--context",
  "--after-context",
  "--before-context",
  "--include",
  "--exclude",
  "--exclude-dir",
]);

/**
 * rg options that take the next word as their value. rg's `-r`
 * (replace) and `-E` (encoding) take one; grep's `-r` and `-E` are flags,
 * so the two tools have separate tables.
 */
const RG_OPTIONS_WITH_VALUE: ReadonlySet<string> = new Set([
  "-A",
  "-B",
  "-C",
  "-m",
  "-g",
  "-t",
  "-T",
  "-j",
  "-M",
  "-r",
  "-E",
  "--glob",
  "--type",
  "--type-not",
  "--max-count",
  "--context",
  "--after-context",
  "--before-context",
  "--max-depth",
  "--replace",
  "--color",
  "--sort",
  "--encoding",
]);

/**
 * Search commands whose pattern {@link bashSearchPatterns} reads, with
 * the options that take a value. Long options written `--name=value` are
 * one word and need no entry.
 */
const BASH_SEARCH_COMMANDS: ReadonlyMap<string, ReadonlySet<string>> = new Map([
  ["grep", GREP_OPTIONS_WITH_VALUE],
  ["egrep", GREP_OPTIONS_WITH_VALUE],
  ["fgrep", GREP_OPTIONS_WITH_VALUE],
  ["rg", RG_OPTIONS_WITH_VALUE],
]);

/** `--regexp=<pattern>` written as one word. */
const REGEXP_INLINE = "--regexp=";

/** How a file-reading command's words map to the files it opens. */
type FileReader = {
  /** Options whose next word is their value, not a file. */
  valued: ReadonlySet<string>;
  /**
   * Options that give the script, so no positional script follows. Set
   * only for commands whose first positional is otherwise the script.
   */
  scriptFlags?: ReadonlySet<string>;
};

const PLAIN_READER: FileReader = { valued: new Set() };

/**
 * Reader commands whose operands are files they print or measure, as
 * {@link bashFileOperands} reads them. Listings and searches (`ls`,
 * `find`, `grep`) are left out: they do not open a file for its content.
 */
const BASH_FILE_READERS: ReadonlyMap<string, FileReader> = new Map([
  ["cat", PLAIN_READER],
  ["nl", PLAIN_READER],
  ["tac", PLAIN_READER],
  ["strings", PLAIN_READER],
  ["wc", PLAIN_READER],
  ["uniq", PLAIN_READER],
  ["diff", PLAIN_READER],
  ["cmp", PLAIN_READER],
  ["head", { valued: new Set(["-n", "-c"]) }],
  ["tail", { valued: new Set(["-n", "-c"]) }],
  ["cut", { valued: new Set(["-d", "-f", "-c", "-b"]) }],
  ["sort", { valued: new Set(["-k", "-t"]) }],
  ["od", { valued: new Set(["-t", "-N", "-j", "-A"]) }],
  ["xxd", { valued: new Set(["-l", "-s", "-c", "-g"]) }],
  ["hexdump", { valued: new Set(["-n", "-s", "-e"]) }],
  [
    "sed",
    {
      valued: new Set(["-e", "-f", "--expression", "--file"]),
      scriptFlags: new Set(["-e", "-f", "--expression", "--file"]),
    },
  ],
  [
    "awk",
    { valued: new Set(["-F", "-v", "-f"]), scriptFlags: new Set(["-f"]) },
  ],
  ["jq", { valued: new Set(["-f"]), scriptFlags: new Set(["-f"]) }],
]);

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
  for (const words of stageCommands(command)) {
    const valued = BASH_SEARCH_COMMANDS.get(words[0]?.text ?? "");
    if (!valued) continue;
    const args = words.slice(1).map((word) => word.text);
    patterns.push(...searchPatterns(args, valued));
  }
  return patterns;
}

/**
 * The files the reader stages of a bash command open for their content
 * (`cat`, `head`, `sed -n`, ...; the table above), in order. A stage
 * counts only when `classifyBashCommand`'s rules judge it a reader, so
 * `sed -i` opens nothing. Options, their values, the script of `sed`,
 * `awk`, and `jq`, redirects, and words with `$` or glob characters are
 * skipped. Paths come back as written, relative to whatever directory
 * an earlier `cd` chose.
 */
export function bashFileOperands(command: string): string[] {
  const files: string[] = [];
  for (const words of stageCommands(command)) {
    const reader = BASH_FILE_READERS.get(words[0]?.text ?? "");
    if (reader) files.push(...fileOperands(words.slice(1), reader));
  }
  return files;
}

/**
 * Each stage's words from the command name on, heredoc bodies dropped.
 * Compound stages, and stages of a file-reading command that the reader
 * rules judge a runner (`sed -i`), are skipped.
 */
function stageCommands(command: string): Word[][] {
  const out: Word[][] = [];
  for (const stages of splitParts(withoutHeredocBodies(command))) {
    for (const stage of stages) {
      const words = commandWords(stage);
      if (words === "compound" || words.length === 0) continue;
      if (
        BASH_FILE_READERS.has(words[0]?.text ?? "") &&
        !isReaderStage(stage)
      ) {
        continue;
      }
      out.push(words);
    }
  }
  return out;
}

/** Patterns given to one search stage, from its words after the name. */
function searchPatterns(
  args: readonly string[],
  valued: ReadonlySet<string>,
): string[] {
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
      if (valued.has(word)) index += 1;
      continue;
    }
    return explicit.length > 0 ? explicit : [word];
  }
  return explicit;
}

/** The file operands among one reader stage's words after the name. */
function fileOperands(args: readonly Word[], reader: FileReader): string[] {
  const files: string[] = [];
  let scriptPending = reader.scriptFlags !== undefined;
  for (let index = 0; index < args.length; index += 1) {
    const word = args[index];
    if (!word) continue;
    if (/[<>]/.test(word.bare)) {
      // A bare `>` or `2>` takes the next word as its target.
      if (/^\d*[<>]+&?$/.test(word.bare)) index += 1;
      continue;
    }
    if (word.bare.startsWith("-")) {
      if (reader.scriptFlags?.has(word.text)) scriptPending = false;
      if (reader.valued.has(word.text)) index += 1;
      continue;
    }
    if (scriptPending) {
      scriptPending = false;
      continue;
    }
    if (/[$*?[]/.test(word.bare)) continue;
    files.push(word.text);
  }
  return files;
}
