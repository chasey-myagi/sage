/**
 * Bash command classification: whether a `bash` call only reads the
 * workspace (`reader`), runs something (`runner`), or cannot be told apart
 * (`mixed`).
 *
 * Contract: every export is a pure function of the command text.
 * It never runs, fully parses, or expands the command. One quote-aware scan
 * splits it into parts, pipeline stages, and words; every operator,
 * redirect, and comment is recognized only outside quotes. Each stage is
 * judged by its first word against the tables below. Anything not listed
 * as a reader is a runner, so an unknown tool is never mistaken for a read.
 *
 * Intentionally not in the Log: the classification. Callers that act on it
 * record their own reason.
 */

/** What a bash command does to the workspace, judged from its text. */
export type BashKind = "reader" | "runner" | "mixed";

/** A part's or stage's kind; `neutral` ones do not affect the result. */
type PartKind = BashKind | "neutral";

/**
 * One shell word. `text` has quotes removed and escapes resolved; `bare` is
 * the same length with every quoted or escaped character replaced by
 * `QUOTED_MASK`, so operator checks on `bare` never see quoted text.
 */
export type Word = { text: string; bare: string };

/** Stands in for a quoted or escaped character in `Word.bare`. */
const QUOTED_MASK = "_";

/**
 * Commands whose output is file content, a listing, or search hits: the
 * same evidence `read`, `grep`, and `ls` give. `sed`, `sort`, and `find` are
 * readers only without `SED_WRITE_FLAG`, `SORT_OUTPUT_FLAG`, and
 * `FIND_ACTION_FLAGS`; `git` is judged by `GIT_READER_SUBCOMMANDS`. Pagers
 * (`less`, `more`) are left out because they need a terminal.
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
 * neither read the workspace nor run anything, so a stage made of one is
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
 * Prefix commands that run the next word as the command: the stage is
 * judged by that word. Their options (words starting with `-`, plus the
 * value after one in `WRAPPER_OPTIONS_WITH_VALUE`) and `env`'s `NAME=value`
 * words are skipped first. `xargs grep x` is a reader, `sudo make` a
 * runner.
 */
const BASH_WRAPPER_COMMANDS: ReadonlySet<string> = new Set([
  "sudo",
  "env",
  "time",
  "nice",
  "xargs",
  "command",
  "exec",
]);

/** Wrapper options that take the next word as their value. */
const WRAPPER_OPTIONS_WITH_VALUE: ReadonlySet<string> = new Set([
  "-n",
  "-u",
  "-g",
  "-I",
  "-P",
  "-L",
  "-d",
  "-s",
  "-E",
]);

/**
 * Words that open a compound command. Its body runs once per iteration or
 * branch, which the text alone cannot tell, so the stage is `mixed`.
 */
const BASH_COMPOUND_KEYWORDS: ReadonlySet<string> = new Set([
  "for",
  "if",
  "while",
  "until",
  "case",
  "select",
  "function",
]);

/**
 * Words that continue or close a compound command. They carry no command
 * of their own, so they are skipped and the next word is judged.
 */
const BASH_FILLER_KEYWORDS: ReadonlySet<string> = new Set([
  "do",
  "done",
  "then",
  "else",
  "elif",
  "fi",
  "esac",
  "!",
]);

/** How a `git` subcommand qualifies as a reader. */
type GitReadRule =
  /** Always prints without changing the repository. */
  | "always"
  /** Lists only when no positional argument follows (`git branch -a`). */
  | "listing"
  /** Reads only when its first argument is one of these (`git stash list`). */
  | ReadonlySet<string>;

/**
 * `git` subcommands that print history, diffs, refs, or tracked content
 * without changing the repository. `branch`, `tag`, and `remote` create or
 * delete with a positional argument, so only their bare listing forms read.
 * Every other subcommand (commit, checkout, add, apply, push, ...) is a
 * runner.
 */
const GIT_READER_SUBCOMMANDS: ReadonlyMap<string, GitReadRule> = new Map<
  string,
  GitReadRule
>([
  ["show", "always"],
  ["diff", "always"],
  ["log", "always"],
  ["status", "always"],
  ["blame", "always"],
  ["grep", "always"],
  ["ls-files", "always"],
  ["ls-tree", "always"],
  ["rev-parse", "always"],
  ["cat-file", "always"],
  ["shortlog", "always"],
  ["describe", "always"],
  ["branch", "listing"],
  ["tag", "listing"],
  ["remote", "listing"],
  ["worktree", new Set(["list"])],
  ["config", new Set(["--get", "--get-all", "--get-regexp", "--list", "-l"])],
  ["stash", new Set(["list", "show"])],
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

/** A `sort` flag that writes the result to a file: `-o`, `--output`. */
const SORT_OUTPUT_FLAG = /^(?:-[A-Za-z]*o|--output)/;

/**
 * The shell option that makes a pipeline fail when any stage fails. Without
 * it the exit status is the last stage's, so a runner piped into `tail`
 * reports success even when it failed.
 */
const PIPEFAIL_OPTION = "pipefail";

/** The builtin that turns `pipefail` on. */
const SET_COMMAND = "set";

/** A leading `NAME=value` word, skipped before the command name. */
const ENV_ASSIGNMENT = /^[A-Za-z_][A-Za-z0-9_]*=/;

/**
 * A heredoc opener such as `<<EOF`, `<<-'EOF'`, or `<<"EOF"`. The body up
 * to the closing word is data, not commands, and is dropped.
 */
const HEREDOC_START = /<<-?\s*(['"]?)([A-Za-z_][A-Za-z0-9_]*)\1/;

/**
 * Classifies a bash command.
 *
 * Normalization: heredoc bodies are dropped; outside quotes, `#` at the
 * start of a word comments out the rest of the line, `&&`, `||`, `;`, and
 * newlines split parts, `|` splits pipeline stages, and whitespace splits
 * words. A stage drops grouping `(`/`{`, leading `NAME=value` words,
 * compound-command filler (`do`, `then`, ...), and wrapper commands
 * (`sudo`, `xargs`, ...), then is judged by its first word. An unquoted
 * output redirect to a file (`>`, `>>`, `&>`; not `2>`, `>&2`, or
 * `/dev/null`) makes the stage a runner.
 *
 * A pipeline is judged by its first stage plus any later runner or mixed
 * stage; later reader stages only filter (`cat a | grep x` reads). A
 * runner piped onward is `mixed` unless the command sets `pipefail`,
 * because the pipeline's exit status hides the runner's failure. The
 * command is `reader` when every part reads (or only neutral parts remain),
 * `runner` when every part runs, and `mixed` otherwise.
 */
export function classifyBashCommand(command: string): BashKind {
  const parts = splitParts(withoutHeredocBodies(command));
  const kinds: BashKind[] = [];
  let pipefail = false;
  for (const stages of parts) {
    const kind = pipelineKind(stages, pipefail);
    if (kind !== "neutral") kinds.push(kind);
    pipefail ||= setsPipefail(stages);
  }
  return combine(kinds);
}

/**
 * Whether a part is a `set` that turns on `pipefail` outside quotes. It
 * only affects the parts after it, so a later `set -o pipefail` or the word
 * inside an argument does not count.
 */
function setsPipefail(stages: readonly Word[][]): boolean {
  return stages.some(
    (words) =>
      words[0]?.text === SET_COMMAND &&
      words.some((w) => w.bare === PIPEFAIL_OPTION),
  );
}

/** `reader` or `runner` when every kind agrees (an empty list reads). */
function combine(kinds: readonly BashKind[]): BashKind {
  if (kinds.every((kind) => kind === "reader")) return "reader";
  if (kinds.every((kind) => kind === "runner")) return "runner";
  return "mixed";
}

/**
 * `command` with every heredoc body, from the line after its opener to
 * the closing word's line, removed.
 */
export function withoutHeredocBodies(command: string): string {
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

/** Accumulates words into stages and stages into parts during one scan. */
function createPartsBuilder() {
  const parts: Word[][][] = [];
  let stages: Word[][] = [];
  let words: Word[] = [];
  let word: Word = { text: "", bare: "" };
  let started = false;
  const builder = {
    add(char: string, quoted: boolean) {
      word.text += char;
      word.bare += quoted ? QUOTED_MASK : char;
      started = true;
    },
    start() {
      started = true;
    },
    isWordEmpty: () => !started,
    endWord() {
      if (started) words.push(word);
      word = { text: "", bare: "" };
      started = false;
    },
    endStage() {
      builder.endWord();
      stages.push(words);
      words = [];
    },
    endPart() {
      builder.endStage();
      parts.push(stages);
      stages = [];
    },
    finish(): Word[][][] {
      builder.endPart();
      return parts;
    },
  };
  return builder;
}

/**
 * Splits into parts (`&&`, `||`, `;`, newline), each a list of pipeline
 * stages (`|`, `|&`), each a list of words. Quotes and backslash escapes
 * keep operators, `#`, and `>` literal.
 */
export function splitParts(command: string): Word[][][] {
  const out = createPartsBuilder();
  let quote: "'" | '"' | undefined;
  for (let index = 0; index < command.length; index += 1) {
    const char = command[index] ?? "";
    const next = command[index + 1];
    if (quote !== undefined) {
      if (char === quote) quote = undefined;
      else if (char === "\\" && quote === '"' && next !== undefined) {
        out.add(next, true);
        index += 1;
      } else out.add(char, true);
      continue;
    }
    if (char === "'" || char === '"') {
      quote = char;
      out.start();
      continue;
    }
    if (char === "\\") {
      if (next === "\n") out.endWord();
      else if (next !== undefined) out.add(next, true);
      index += 1;
      continue;
    }
    if (char === "#" && out.isWordEmpty()) {
      while (index + 1 < command.length && command[index + 1] !== "\n") {
        index += 1;
      }
      continue;
    }
    if (char === " " || char === "\t" || char === "\r") {
      out.endWord();
      continue;
    }
    if ((char === "&" && next === "&") || (char === "|" && next === "|")) {
      out.endPart();
      index += 1;
      continue;
    }
    if (char === ";" || char === "\n") {
      out.endPart();
      continue;
    }
    if (char === "|") {
      out.endStage();
      if (next === "&") index += 1;
      continue;
    }
    out.add(char, false);
  }
  return out.finish();
}

function pipelineKind(stages: readonly Word[][], pipefail: boolean): PartKind {
  const [first = "neutral", ...rest] = stages.map(stageKind);
  if (first === "runner" && rest.length > 0 && !pipefail) return "mixed";
  const kinds = [first, ...rest.filter((kind) => kind !== "reader")];
  const relevant = kinds.filter((kind) => kind !== "neutral");
  if (relevant.length === 0) return "neutral";
  return combine(relevant);
}

/** Whether one pipeline stage is judged a reader, by the rules above. */
export function isReaderStage(stage: readonly Word[]): boolean {
  return stageKind(stage) === "reader";
}

function stageKind(stage: readonly Word[]): PartKind {
  const words = commandWords(stage);
  if (words === "compound") return "mixed";
  const name = words[0]?.text;
  if (name === undefined) return "neutral";
  if (writesFile(stage)) return "runner";
  if (BASH_NEUTRAL_COMMANDS.has(name)) return "neutral";
  if (!BASH_READER_COMMANDS.has(name)) return "runner";
  const args = words.slice(1).map((word) => word.text);
  const flagged = (pattern: RegExp) => args.some((arg) => pattern.test(arg));
  if (name === "sed" && flagged(SED_WRITE_FLAG)) return "runner";
  if (name === "sort" && flagged(SORT_OUTPUT_FLAG)) return "runner";
  if (name === "find" && args.some((arg) => FIND_ACTION_FLAGS.has(arg))) {
    return "runner";
  }
  if (name === "git") return gitKind(args);
  return "reader";
}

/**
 * The stage's words from the command name on: grouping, assignments,
 * filler keywords, and wrappers removed. `compound` when the stage opens a
 * compound command.
 */
export function commandWords(stage: readonly Word[]): Word[] | "compound" {
  let words = withoutGrouping(stage);
  for (;;) {
    words = withoutAssignments(words);
    const name = words[0]?.text;
    if (name === undefined) return words;
    if (BASH_COMPOUND_KEYWORDS.has(name)) return "compound";
    if (BASH_FILLER_KEYWORDS.has(name)) {
      words = words.slice(1);
      continue;
    }
    if (!BASH_WRAPPER_COMMANDS.has(name)) return words;
    words = withoutOptions(words.slice(1));
  }
}

function withoutGrouping(stage: readonly Word[]): Word[] {
  const words = stage.map((word) => ({ ...word }));
  const first = words[0];
  if (first && /^[({]/.test(first.bare)) {
    first.text = first.text.slice(1);
    first.bare = first.bare.slice(1);
  }
  const last = words.at(-1);
  if (last && /[)}]$/.test(last.bare)) {
    last.text = last.text.slice(0, -1);
    last.bare = last.bare.slice(0, -1);
  }
  return words.filter((word) => word.bare.length > 0);
}

function withoutAssignments(words: readonly Word[]): Word[] {
  let start = 0;
  while (
    start < words.length &&
    ENV_ASSIGNMENT.test(words[start]?.bare ?? "")
  ) {
    start += 1;
  }
  return words.slice(start);
}

function withoutOptions(words: readonly Word[]): Word[] {
  let start = 0;
  while (start < words.length) {
    const bare = words[start]?.bare ?? "";
    if (!bare.startsWith("-")) break;
    start += WRAPPER_OPTIONS_WITH_VALUE.has(bare) ? 2 : 1;
  }
  return words.slice(start);
}

/**
 * True when an unquoted output redirect writes a file. `2>` (diagnostics),
 * descriptor duplication (`2>&1`, `>&2`), and `/dev/null` are not writes.
 */
function writesFile(stage: readonly Word[]): boolean {
  for (let index = 0; index < stage.length; index += 1) {
    const word = stage[index];
    if (!word) continue;
    const at = word.bare.indexOf(">");
    if (at < 0) continue;
    const prefix = word.bare.slice(0, at);
    if (prefix === "2") continue;
    let rest = word.text.slice(at + 1);
    if (word.bare[at + 1] === ">") rest = rest.slice(1);
    if (rest.startsWith("&")) continue;
    const target = rest.length > 0 ? rest : (stage[index + 1]?.text ?? "");
    if (target.length === 0 || target === "/dev/null") continue;
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
    const rule = GIT_READER_SUBCOMMANDS.get(word);
    const after = args.slice(index + 1);
    return isGitRead(rule, after) ? "reader" : "runner";
  }
  return "runner";
}

function isGitRead(
  rule: GitReadRule | undefined,
  after: readonly string[],
): boolean {
  if (rule === undefined) return false;
  if (rule === "always") return true;
  if (rule === "listing") return after.every((arg) => arg.startsWith("-"));
  return rule.has(after[0] ?? "");
}
