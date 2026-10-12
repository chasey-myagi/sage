/**
 * `sage calibrate-jev <session.jsonl…> --out <dir>`: asks Jev, offline,
 * about every tool chunk of recorded sessions exactly as the entry-mode
 * jev scorer would have, then sweeps the tier thresholds against whether
 * each chunk was needed later. The main model is never called.
 *
 * Contract: before any request the uncached requests' estimated input
 * tokens are converted at `jevInputUsdPerMTok`; when the spend already in
 * `<out>/spend.json` plus that estimate is above `--max-usd` (default
 * `DEFAULT_MAX_USD`) the command exits 2 without sending, and `--dry-run`
 * prints the estimate and exits 0 without sending or needing a key. A run
 * writes `<out>/cache/`, `<out>/spend.json`, and `<out>/sweep.json`, then
 * prints one JSON summary line with the recommended thresholds. Exit 0
 * when every request was answered, 2 when cumulative spend passed
 * `--max-usd` mid-run, 1 when a request failed or the rate-limit breaker
 * opened, for a missing key, a bad argument (with the usage), or an
 * unreadable file. Outputs hold ids, counts, and prices, never chunk text.
 *
 * Intentionally not in the Log: everything this command reads or writes.
 * The recorded sessions are only read.
 */

import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { errorText, JsonlLogStore, type StoredEntry } from "@sage/core";
import {
  type EntryJevRequest,
  type EntryJevScan,
  entryJevRequests,
  estimateRequestTokens,
  JEV_MODEL_ID,
  type JevEndpoint,
  ratesFor,
  readJudgment,
  requestBody,
} from "@sage/plugins";
import {
  type CachedAnswer,
  jevInputUsdPerMTok,
  jevUsd,
  readCached,
  readSpend,
  requestKey,
  type SendOutcome,
  type Spend,
  sendRequests,
  sumSpend,
} from "./calibrate-send.js";
import {
  DEFAULT_SWEEP_GRID,
  type LabeledChunk,
  recommendRow,
  type SweepGrid,
  type SweepRow,
  sweepThresholds,
} from "./calibrate-sweep.js";
import type { CommandIo } from "./command-io.js";
import {
  type Checked,
  type CommandFlags,
  checkProbabilities,
  checkUsd,
  type FlagError,
  type OptionName,
  readFlags,
  renderUsage,
  usageError,
} from "./options.js";
import { type JevFetch, jevApiKey } from "./scorers.js";
import { neededLaterIn } from "./stats.js";

/**
 * Default `--max-usd`: one dollar, far above a calibration over a few
 * dozen sessions, low enough that a wrong glob cannot run up a bill.
 */
export const DEFAULT_MAX_USD = 1;
/**
 * Per-attempt timeout in milliseconds. Offline nothing waits on the
 * answer, so it is looser than the scorer's in-turn timeout.
 */
const REQUEST_TIMEOUT_MS = 30_000;

/** Flags `sage calibrate-jev` accepts; session files are positionals. */
const CALIBRATE_FLAGS = [
  { name: "out", usage: "required" },
  { name: "max-usd", usage: "optional" },
  { name: "dry-run", usage: "optional" },
  { name: "t-full", usage: "optional" },
  { name: "t-none", usage: "optional" },
  { name: "t-low", usage: "optional" },
  { name: "t-keep", usage: "optional" },
] as const satisfies CommandFlags<OptionName>;

type GridFlag = "t-full" | "t-none" | "t-low" | "t-keep";

const GRID_KEYS: Readonly<Record<GridFlag, keyof SweepGrid>> = {
  "t-full": "full",
  "t-none": "none",
  "t-low": "low",
  "t-keep": "keep",
};

type CalibrateOptions = {
  files: string[];
  out: string;
  maxUsd: number;
  dryRun: boolean;
  grid: SweepGrid;
};

/** One chunk to ask about: its rebuilt request, cache key, body, and label. */
type Planned = {
  /** Basename of the session file. */
  session: string;
  entry: EntryJevRequest;
  key: string;
  body: string;
  needed: boolean;
};

/** What `runCalibrateJev` reads from the process. */
export type CalibrateIo = CommandIo & {
  env: NodeJS.ProcessEnv;
  /** Test seam. Production uses the global `fetch`. */
  jevFetch?: JevFetch;
};

/** The `sage calibrate-jev` usage line, without the leading `usage:`. */
export function calibrateUsage(): string {
  return `${renderUsage("sage calibrate-jev", CALIBRATE_FLAGS)} <session.jsonl…>`;
}

/** Runs `sage calibrate-jev <argv>` and returns the process exit code. */
export async function runCalibrateJev(
  argv: string[],
  io: CalibrateIo,
): Promise<number> {
  const parsed = parseCalibrateArgs(argv, io.cwd);
  if (!parsed.ok) {
    io.stderr.write(`${parsed.message}\n`);
    return parsed.exitCode;
  }
  const opts = parsed.value;
  const missing = opts.files.find((file) => !existsSync(file));
  if (missing) {
    io.stderr.write(`Missing session file: ${missing}\n`);
    return 1;
  }
  try {
    return await calibrate(opts, io);
  } catch (error) {
    io.stderr.write(`${errorText(error)}\n`);
    return 1;
  }
}

async function calibrate(
  opts: CalibrateOptions,
  io: CalibrateIo,
): Promise<number> {
  const usdPerMTok = jevInputUsdPerMTok();
  const model = io.env.JEV_MODEL || JEV_MODEL_ID;
  const scans = await Promise.all(
    opts.files.map((file) => planSession(file, model)),
  );
  const planned = scans.flatMap((scan) => scan.planned);
  const split = splitCached(planned, path.join(opts.out, "cache"), usdPerMTok);
  const prior = readSpend(path.join(opts.out, "spend.json"), usdPerMTok);
  const counts = {
    sessions: opts.files.length,
    chunks: planned.length,
    cached: planned.length - split.pending.size,
  };
  const { estimate } = split;
  const projectedUsd = prior.usd + estimate.usd;
  if (opts.dryRun) {
    const report = {
      dryRun: true,
      ...counts,
      pending: split.pending.size,
      estimate,
      priorUsd: prior.usd,
      maxUsd: opts.maxUsd,
      withinBudget: projectedUsd <= opts.maxUsd,
    };
    io.stdout.write(`${JSON.stringify(report)}\n`);
    return 0;
  }
  io.stderr.write(
    `calibrate-jev: ${split.pending.size} uncached requests, ~${estimate.inputTokens} input tokens, ~${formatUsd(estimate.usd)}; spent so far ${formatUsd(prior.usd)}\n`,
  );
  if (projectedUsd > opts.maxUsd) {
    io.stderr.write(
      `Spent ${formatUsd(prior.usd)} plus estimated ${formatUsd(estimate.usd)} exceeds --max-usd ${opts.maxUsd}; raise --max-usd or check the estimate with --dry-run\n`,
    );
    return 2;
  }
  const sent = await sendPending(split.pending, {
    opts,
    io,
    prior,
    usdPerMTok,
  });
  if (typeof sent === "string") {
    io.stderr.write(`${sent}\n`);
    return 1;
  }
  for (const [key, answer] of sent.answers) split.answers.set(key, answer);
  const labeled = labelChunks(planned, split.answers);
  const recommended = writeSweep(labeled.chunks, opts);
  const total = sumSpend(prior, sent.spent, usdPerMTok);
  reportStop(sent, total, io);
  const summary = {
    ...counts,
    sent: sent.spent.requests + sent.failed,
    failed: sent.failed,
    unsent: sent.unsent,
    skipped: { ...sumSkipped(scans), unanswered: labeled.unanswered },
    unpriced: labeled.chunks.filter((chunk) => chunk.rates.source === "default")
      .length,
    estimate,
    spent: { inputTokens: sent.spent.inputTokens, usd: sent.spent.usd },
    totalSpentUsd: total.usd,
    stopped: sent.stopped ?? null,
    recommended,
  };
  io.stdout.write(`${JSON.stringify(summary)}\n`);
  return exitCodeFor(sent);
}

/** Sweeps `chunks`, writes `<out>/sweep.json`, and returns the recommended row. */
function writeSweep(
  chunks: readonly LabeledChunk[],
  opts: CalibrateOptions,
): SweepRow | null {
  const rows = sweepThresholds(chunks, opts.grid);
  const recommended = recommendRow(rows) ?? null;
  const sweep = {
    grid: opts.grid,
    chunks: chunks.map(chunkRow),
    rows,
    recommended,
  };
  writeFileSync(
    path.join(opts.out, "sweep.json"),
    `${JSON.stringify(sweep)}\n`,
  );
  return recommended;
}

/**
 * Cached answers by key, and the uncached bodies with their estimated
 * input tokens and USD; a body two chunks share is sent and counted once.
 */
function splitCached(
  planned: readonly Planned[],
  cacheDir: string,
  usdPerMTok: number,
): {
  answers: Map<string, CachedAnswer>;
  pending: Map<string, string>;
  estimate: { inputTokens: number; usd: number };
} {
  const answers = new Map<string, CachedAnswer>();
  const pending = new Map<string, string>();
  let inputTokens = 0;
  for (const item of planned) {
    if (answers.has(item.key) || pending.has(item.key)) continue;
    const hit = readCached(cacheDir, item.key);
    if (hit) {
      answers.set(item.key, hit);
      continue;
    }
    pending.set(item.key, item.body);
    inputTokens += estimateRequestTokens(item.entry.request);
  }
  return {
    answers,
    pending,
    estimate: { inputTokens, usd: jevUsd(inputTokens, usdPerMTok) },
  };
}

function sumSkipped(
  scans: readonly { skipped: EntryJevScan["skipped"] }[],
): EntryJevScan["skipped"] {
  const total = { isError: 0, unsendable: 0 };
  for (const scan of scans) {
    total.isError += scan.skipped.isError;
    total.unsendable += scan.skipped.unsendable;
  }
  return total;
}

function reportStop(sent: SendOutcome, total: Spend, io: CalibrateIo): void {
  const kept = `kept ${sent.answers.size} answers from this run`;
  if (sent.failed > 0) {
    io.stderr.write(`${sent.failed} Jev requests failed; the rest were sent\n`);
  }
  if (sent.stopped?.kind === "budget") {
    io.stderr.write(
      `Jev spend ${formatUsd(total.usd)} in total passed --max-usd; stopped sending, ${kept}\n`,
    );
  } else if (sent.stopped?.kind === "rate-limited") {
    io.stderr.write(
      `Jev stayed rate-limited (${sent.stopped.fallback}); stopped sending, ${kept}\n`,
    );
  }
}

function formatUsd(usd: number): string {
  return `$${usd.toFixed(4)}`;
}

/** Sends the uncached requests, or returns the stderr line when no key is set. */
async function sendPending(
  pending: ReadonlyMap<string, string>,
  run: {
    opts: CalibrateOptions;
    io: CalibrateIo;
    prior: Spend;
    usdPerMTok: number;
  },
): Promise<SendOutcome | string> {
  const { opts, io } = run;
  const apiKey = jevApiKey(io.env);
  if (pending.size > 0 && !apiKey) {
    return "calibrate-jev needs JEV_API_KEY or TYPESAFE_API_KEY to send uncached requests";
  }
  mkdirSync(opts.out, { recursive: true });
  const baseUrl = io.env.JEV_BASE_URL;
  const endpoint: JevEndpoint = {
    apiKey: apiKey ?? "",
    ...(baseUrl ? { baseUrl } : {}),
    timeoutMs: REQUEST_TIMEOUT_MS,
    ...(io.jevFetch ? { fetchImpl: io.jevFetch } : {}),
  };
  return sendRequests(pending, {
    endpoint,
    cacheDir: path.join(opts.out, "cache"),
    spendFile: path.join(opts.out, "spend.json"),
    prior: run.prior,
    maxUsd: opts.maxUsd,
    usdPerMTok: run.usdPerMTok,
  });
}

function exitCodeFor(sent: SendOutcome): number {
  if (sent.stopped?.kind === "budget") return 2;
  if (sent.stopped?.kind === "rate-limited" || sent.failed > 0) return 1;
  return 0;
}

/** Rebuilds one session's requests and labels them. */
async function planSession(
  file: string,
  model: string,
): Promise<{ planned: Planned[]; skipped: EntryJevScan["skipped"] }> {
  const entries = readSession(file);
  const scan = await entryJevRequests(entries);
  const isNeeded = neededLaterIn(entries);
  const planned = scan.requests.map((entry) => {
    const body = requestBody(entry.request, model);
    return {
      session: path.basename(file),
      entry,
      key: requestKey(body),
      body,
      needed: isNeeded(entry.chunkId, entry.decidedAt, entry.turn),
    };
  });
  return { planned, skipped: scan.skipped };
}

function readSession(file: string): StoredEntry[] {
  const store = new JsonlLogStore({ dir: path.dirname(file) });
  return store.path(path.basename(file, ".jsonl"));
}

/** Chunks whose answer carries every fixed judgment, in plan order. */
function labelChunks(
  planned: readonly Planned[],
  answers: ReadonlyMap<string, CachedAnswer>,
): { chunks: LabeledChunk[]; unanswered: number } {
  const chunks: LabeledChunk[] = [];
  let unanswered = 0;
  for (const item of planned) {
    const answer = answers.get(item.key);
    const candidates = item.entry.request.candidates;
    const judgment = answer && readJudgment(answer.answers, candidates.length);
    if (!judgment) {
      unanswered += 1;
      continue;
    }
    chunks.push({
      session: item.session,
      chunkId: item.entry.chunkId,
      tokens: item.entry.tokens,
      remainingTurns: item.entry.remainingTurns,
      needed: item.needed,
      rates: ratesFor(item.entry.pricing),
      judgment,
      candidates,
    });
  }
  return { chunks, unanswered };
}

/** A chunk as `sweep.json` lists it: ids and numbers, no text. */
function chunkRow(chunk: LabeledChunk): Record<string, unknown> {
  return {
    session: chunk.session,
    chunkId: chunk.chunkId,
    tokens: chunk.tokens,
    remainingTurns: chunk.remainingTurns,
    needed: chunk.needed,
  };
}

function parseCalibrateArgs(
  argv: readonly string[],
  cwd: string,
): Checked<CalibrateOptions> {
  const usage = `usage: ${calibrateUsage()}`;
  const opts: CalibrateOptions = {
    files: [],
    out: "",
    maxUsd: DEFAULT_MAX_USD,
    dryRun: false,
    grid: { ...DEFAULT_SWEEP_GRID },
  };
  for (const token of readFlags(argv, CALIBRATE_FLAGS)) {
    if (token.kind === "invalid") return usageError(usage);
    if (token.kind === "positional") {
      if (!token.value.endsWith(".jsonl")) return usageError(usage);
      opts.files.push(path.resolve(cwd, token.value));
      continue;
    }
    if (token.kind === "switch") {
      if (token.name !== "dry-run") return usageError(usage);
      opts.dryRun = true;
      continue;
    }
    const applied = applyValue(opts, token.name, token.value, cwd);
    if (applied) return applied;
  }
  if (opts.files.length === 0 || !opts.out) return usageError(usage);
  return { ok: true, value: opts };
}

/** Sets one string flag on `opts`; returns the error for a bad value. */
function applyValue(
  opts: CalibrateOptions,
  name: (typeof CALIBRATE_FLAGS)[number]["name"],
  raw: string,
  cwd: string,
): FlagError | undefined {
  if (name === "out") {
    opts.out = path.resolve(cwd, raw);
    return undefined;
  }
  if (name === "max-usd") {
    const checked = checkUsd(name, raw);
    if (!checked.ok) return checked;
    opts.maxUsd = checked.value;
    return undefined;
  }
  // readFlags turns `--dry-run=x` into an invalid token, so this only
  // narrows the name for the threshold flags below.
  if (name === "dry-run") return usageError(`usage: ${calibrateUsage()}`);
  const checked = checkProbabilities(name, raw);
  if (!checked.ok) return checked;
  opts.grid[GRID_KEYS[name]] = checked.value;
  return undefined;
}
