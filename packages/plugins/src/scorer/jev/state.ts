/**
 * The state and questions of one jev request: one chunk, the task, the
 * ledger, and the assistant prose around the call, sized to Jev's limits.
 *
 * Guarantees: the estimated tokens of the state plus its longest question
 * stay at or below `PAIR_TOKEN_LIMIT`, and of the state plus every
 * question at or below `TOTAL_TOKEN_LIMIT`. The estimate is conservative
 * (`BYTES_PER_TOKEN` UTF-8 bytes per token), so a real tokenizer counts no
 * more. Head and tail are cut before candidate lines; a chunk that cannot
 * fit even with neither yields no request.
 *
 * Intentionally not in the Log: the state. It is copied from a request the
 * Log already rebuilds.
 */

import { Buffer } from "node:buffer";
import type { Chunk } from "../../compiler/chunks.js";
import { clipEnd, clipStart } from "../../compiler/clip.js";
import { summarizeInput } from "../../compiler/stub.js";
import { candidateLines } from "./candidates.js";
import { type NoulQuestion, questionsFor } from "./questions.js";

/** Jev's limit on the state plus the single longest question, in tokens. */
const PAIR_TOKEN_LIMIT = 32_000;
/** Jev's limit on the state plus all questions, in tokens. */
const TOTAL_TOKEN_LIMIT = 64_000;
/**
 * UTF-8 bytes counted as one token. Real tokenizers average more bytes per
 * token on code and prose; dense text such as hashes or CJK approaches
 * this ratio, so the estimate stays an upper bound.
 */
const BYTES_PER_TOKEN = 2.5;

const TASK_CHAR_LIMIT = 2_000;
const LEDGER_CHAR_LIMIT = 3_000;
/** Assistant prose keeps its end, where the next step is stated. */
const PROSE_CHAR_LIMIT = 1_500;
const INPUT_CHAR_LIMIT = 1_000;
/** Output lines sent before the omitted middle, before any cut for size. */
const HEAD_LINES = 40;
/** Output lines sent after the omitted middle, before any cut for size. */
const TAIL_LINES = 20;
const LINE_CHAR_LIMIT = 400;

/** The chunk as Jev sees it. */
export type ChunkState = {
  id: string;
  tool: string;
  /** Compact JSON of the tool input, at most `INPUT_CHAR_LIMIT` characters. */
  input: string;
  /** The compiler's token estimate of the whole chunk. */
  tokens: number;
  /** Empty, as is `tail`, when `candidates` already hold the whole output. */
  head: string[];
  tail: string[];
  /** Output lines between head and tail that were not sent. */
  omitted_lines: number;
  candidates: string[];
};

/** The whole state of one request. Empty strings stand for absent text. */
export type JevState = {
  task: string;
  ledger: string;
  recent_assistant: string;
  chunk: ChunkState;
};

/** One request's state and questions, and the candidates `keep_<i>` indexes. */
export type ChunkRequest = {
  state: JevState;
  questions: Record<string, NoulQuestion>;
  candidates: string[];
};

/** Text around the chunk, before clipping. */
export type ChunkContext = {
  task: string;
  ledger: string;
  prose: string;
};

/**
 * Builds the request for `chunk` whose original output is `text`, or
 * undefined when no cut fits the limits. Head and tail halve together
 * until both are empty; only then do the candidates halve, and head and
 * tail start over at full size. When every output line is a candidate
 * (`coversOutput`), head and tail are empty and `omitted_lines` is 0.
 */
export function chunkRequest(
  chunk: Chunk,
  text: string,
  context: ChunkContext,
): ChunkRequest | undefined {
  const lines = text.length === 0 ? [] : text.split(/\r?\n/);
  const tool = chunk.toolName ?? "";
  const base = {
    task: clipEnd(context.task, TASK_CHAR_LIMIT),
    ledger: clipEnd(context.ledger, LEDGER_CHAR_LIMIT),
    recent_assistant: clipStart(context.prose, PROSE_CHAR_LIMIT),
  };
  const all = candidateLines(tool, chunk.input, lines);
  for (const candidates of halvings(all.lines)) {
    const questions = questionsFor(candidates);
    const covered = all.coversOutput && candidates.length === all.lines.length;
    const sizes: Iterable<[number, number]> = covered
      ? [[0, 0]]
      : excerptSizes();
    for (const [headCount, tailCount] of sizes) {
      const state: JevState = {
        ...base,
        chunk: {
          id: chunk.id,
          tool,
          input: summarizeInput(chunk.input, INPUT_CHAR_LIMIT),
          tokens: chunk.tokens,
          ...(covered
            ? { head: [], tail: [], omitted_lines: 0 }
            : excerpt(lines, headCount, tailCount)),
          candidates,
        },
      };
      if (fitsLimits(state, questions)) {
        return { state, questions, candidates };
      }
    }
  }
  return undefined;
}

/** `items`, then its first half, quarter, and so on, ending with []. */
function* halvings(items: readonly string[]): Generator<string[]> {
  let count = items.length;
  while (count > 0) {
    yield items.slice(0, count);
    count = Math.floor(count / 2);
  }
  yield [];
}

/** Head and tail line counts, halving from the defaults down to 0 and 0. */
function* excerptSizes(): Generator<[number, number]> {
  let head = HEAD_LINES;
  let tail = TAIL_LINES;
  while (head > 0 || tail > 0) {
    yield [head, tail];
    head = Math.floor(head / 2);
    tail = Math.floor(tail / 2);
  }
  yield [0, 0];
}

function excerpt(
  lines: readonly string[],
  headCount: number,
  tailCount: number,
): Pick<ChunkState, "head" | "tail" | "omitted_lines"> {
  const clip = (line: string) => clipEnd(line, LINE_CHAR_LIMIT);
  if (lines.length <= headCount + tailCount) {
    return { head: lines.map(clip), tail: [], omitted_lines: 0 };
  }
  return {
    head: lines.slice(0, headCount).map(clip),
    tail: lines.slice(lines.length - tailCount).map(clip),
    omitted_lines: lines.length - headCount - tailCount,
  };
}

function fitsLimits(
  state: JevState,
  questions: Record<string, NoulQuestion>,
): boolean {
  const stateTokens = estimateTokens(state);
  let longest = 0;
  let total = 0;
  for (const question of Object.values(questions)) {
    const tokens = estimateTokens(question);
    longest = Math.max(longest, tokens);
    total += tokens;
  }
  return (
    stateTokens + longest <= PAIR_TOKEN_LIMIT &&
    stateTokens + total <= TOTAL_TOKEN_LIMIT
  );
}

function estimateTokens(value: unknown): number {
  return Math.ceil(
    Buffer.byteLength(JSON.stringify(value), "utf8") / BYTES_PER_TOKEN,
  );
}
