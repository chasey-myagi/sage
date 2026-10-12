/**
 * Bash command classification: whether a `bash` call only reads the
 * workspace (`reader`), runs something (`runner`), or both (`mixed`).
 *
 * Contract: `classifyBashCommand` is a pure function of the command text.
 * It never runs, parses fully, or expands the command; it splits on shell
 * control operators outside quotes and judges each part by its first word
 * against the tables below. Anything not listed as a reader is a runner,
 * so an unknown tool is never mistaken for a read.
 *
 * Intentionally not in the Log: the classification. Callers that act on it
 * record their own reason.
 */

/** What a bash command does to the workspace, judged from its text. */
export type BashKind = "reader" | "runner" | "mixed";

/** A part's or stage's kind; `neutral` ones do not affect the result. */
type PartKind = BashKind | "neutral";

/**
 * Commands whose output is file content, a listing, or search hits: the
 * same evidence `read`, `grep`, and `ls` give. `sed` and `find` are readers
 * only without the flags in `SED_WRITE_FLAG` and `FIND_ACTION_FLAGS`; `git`
 * is judged by `GIT_READER_SUBCOMMANDS`. Pagers (`less`, `more`) are left
 * out because they need a terminal.
 */
const BASH_READER_COMMANDS: ReadonlySet<string> = new Set([
  // File content.
  "cat",
  "head",
  "tail",
  "nl",
  "sed",
  "awk",
  "cut",
  "tac",
  "strings",
  "xxd",
  "od",
  "hexdump",
  "jq",
  // Search.
  "grep",
  "egrep",
  "fgrep",
  "rg",
  "ag",
  "ack",
  // Listings and file facts.
  "ls",
  "tree",
  "find",
  "fd",
  "stat",
  "file",
  "du",
  "wc",
  "realpath",
  "readlink",
  "basename",
  "dirname",
  "pwd",
  "which",
  // Comparison and reshaping of what was read.
  "diff",
  "cmp",
  "sort",
  "uniq",
  "comm",
  "tr",
  "column",
  "git",
]);

/**
 * Commands that only move the shell or print their own arguments. They
 * neither read the workspace nor run anything, so a part made of one is
 * dropped: `cd repo && pnpm test` is judged as `pnpm test`, and
 * `pnpm test; echo $?` stays a runner.
 */
const BASH_NEUTRAL_COMMANDS: ReadonlySet<string> = new Set([
  "cd",
  "pushd",
  "popd",
  "echo",
  "printf",
  "true",
  "export",
  "set",
  "unset",
]);

/**
 * `git` subcommands that print history, diffs, or tracked content without
 * changing the repository. Every other subcommand (commit, checkout, add,
 * apply, stash, push, ...) is a runner.
 */
const GIT_READER_SUBCOMMANDS: ReadonlySet<string> = new Set([
  "show",
  "diff",
  "log",
  "status",
  "blame",
  "grep",
  "ls-files",
  "ls-tree",
  "rev-parse",
  "cat-file",
  "shortlog",
  "describe",
]);

/** `git` global options that take the next word as their value. */
const GIT_OPTIONS_WITH_VALUE: ReadonlySet<string> = new Set(["-C", "-c"]);

/** `find` actions that delete files or run another command per match. */
const FIND_ACTION_FLAGS: ReadonlySet<string> = new Set([
  "-exec",
  "-execdir",
  "-ok",
  "-okdir",
  "-delete",
]);

/** A `sed` flag that edits in place: `-i`, `-i.bak`, `-ni`, `--in-place`. */
const SED_WRITE_FLAG = /^(?:-[A-Za-z]*i|--in-place)/;

/**
 * Normalization: every whitespace run becomes one space, so newlines and
 * indentation never change a part's first word.
 */
const WHITESPACE_RUN = /\s+/g;

/** A leading `NAME=value` word, skipped before the command name. */
const ENV_ASSIGNMENT = /^[A-Za-z_][A-Za-z0-9_]*=/;

/**
 * An output redirect to a file: `>`, `>>`, `1>`, `&>` with the target in
 * the same word or the next. `2>&1` and `/dev/null` targets are not file
 * writes.
 */
const OUTPUT_REDIRECT = /^(?:\d|&)?>>?(.*)$/;

/**
 * A heredoc opener such as `<<EOF`, `<<-'EOF'`, or `<<"EOF"`. The body up
 * to the closing word is data, not commands, and is dropped.
 */
const HEREDOC_START = /<<-?\s*(['"]?)([A-Za-z_][A-Za-z0-9_]*)\1/;

/**
 * Classifies a bash command. Normalization drops heredoc bodies, splits on
 * `&&`, `||`, `;`, and newlines into parts and on `|` into pipeline stages
 * (never inside quotes), collapses whitespace, strips grouping `(`/`{`, and
 * skips leading `NAME=value` words. A pipeline is judged by its first stage
 * plus any later runner stage; later reader stages only filter, so
 * `pnpm test | tail` is a runner and `grep -l x | xargs rm` is mixed. The
 * command
 * is `reader` when every part reads (or only neutral parts remain),
 * `runner` when every part runs, and `mixed` otherwise. An output redirect
 * to a file makes that part a runner.
 */
export function classifyBashCommand(command: string): BashKind {
  const kinds = splitParts(withoutHeredocBodies(command))
    .map(pipelineKind)
    .filter((kind) => kind !== "neutral");
  return combine(kinds);
}

/** `reader` or `runner` when every kind agrees (an empty list reads). */
function combine(kinds: readonly BashKind[]): BashKind {
  if (kinds.every((kind) => kind === "reader")) return "reader";
  if (kinds.every((kind) => kind === "runner")) return "runner";
  return "mixed";
}

function withoutHeredocBodies(command: string): string {
  const kept: string[] = [];
  let terminator: string | undefined;
  for (const line of command.split("\n")) {
    if (terminator !== undefined) {
      if (line.trim() === terminator) terminator = undefined;
      continue;
    }
    kept.push(line);
    terminator = HEREDOC_START.exec(line)?.[2];
  }
  return kept.join("\n");
}

/**
 * Splits into parts (`&&`, `||`, `;`, newline), each a list of pipeline
 * stages (`|`, `|&`). Quotes and backslash escapes keep operators literal.
 */
function splitParts(command: string): string[][] {
  const parts: string[][] = [];
  let stages: string[] = [];
  let current = "";
  let quote: "'" | '"' | undefined;
  const endStage = () => {
    stages.push(current);
    current = "";
  };
  const endPart = () => {
    endStage();
    parts.push(stages);
    stages = [];
  };
  for (let index = 0; index < command.length; index += 1) {
    const char = command[index] ?? "";
    const next = command[index + 1];
    if (quote !== undefined) {
      if (char === quote) quote = undefined;
      else if (char === "\\" && quote === '"') {
        current += char + (next ?? "");
        index += 1;
        continue;
      }
      current += char;
      continue;
    }
    if (char === "'" || char === '"') quote = char;
    if (char === "\\") {
      current += char + (next ?? "");
      index += 1;
      continue;
    }
    if ((char === "&" && next === "&") || (char === "|" && next === "|")) {
      endPart();
      index += 1;
      continue;
    }
    if (char === ";" || char === "\n") {
      endPart();
      continue;
    }
    if (char === "|") {
      endStage();
      if (next === "&") index += 1;
      continue;
    }
    current += char;
  }
  endPart();
  return parts;
}

function pipelineKind(stages: readonly string[]): PartKind {
  const [first, ...rest] = stages.map(stageKind);
  const kinds = [first ?? "neutral", ...rest.filter((k) => k === "runner")];
  const relevant = kinds.filter((kind) => kind !== "neutral");
  if (relevant.length === 0) return "neutral";
  return combine(relevant);
}

function stageKind(stage: string): PartKind {
  const words = wordsOf(stage);
  const name = words[0];
  if (name === undefined) return "neutral";
  if (writesFile(words)) return "runner";
  if (BASH_NEUTRAL_COMMANDS.has(name)) return "neutral";
  if (!BASH_READER_COMMANDS.has(name)) return "runner";
  const args = words.slice(1);
  if (name === "sed" && args.some((word) => SED_WRITE_FLAG.test(word))) {
    return "runner";
  }
  if (name === "find" && args.some((word) => FIND_ACTION_FLAGS.has(word))) {
    return "runner";
  }
  if (name === "git") return gitKind(args);
  return "reader";
}

function wordsOf(stage: string): string[] {
  const words = stage
    .replace(WHITESPACE_RUN, " ")
    .trim()
    .replace(/^[({]\s*/, "")
    .replace(/\s*[)}]$/, "")
    .split(" ")
    .filter((word) => word.length > 0);
  let start = 0;
  while (start < words.length && ENV_ASSIGNMENT.test(words[start] ?? "")) {
    start += 1;
  }
  return words.slice(start);
}

function writesFile(words: readonly string[]): boolean {
  for (let index = 0; index < words.length; index += 1) {
    const match = OUTPUT_REDIRECT.exec(words[index] ?? "");
    if (!match) continue;
    const target = match[1] || words[index + 1] || "";
    if (target.startsWith("&") || target === "/dev/null") continue;
    return true;
  }
  return false;
}

function gitKind(args: readonly string[]): PartKind {
  for (let index = 0; index < args.length; index += 1) {
    const word = args[index] ?? "";
    if (GIT_OPTIONS_WITH_VALUE.has(word)) {
      index += 1;
      continue;
    }
    if (word.startsWith("-")) continue;
    return GIT_READER_SUBCOMMANDS.has(word) ? "reader" : "runner";
  }
  return "runner";
}
