import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { isRecord } from "@sage/core";
import {
  createFaux,
  type JsonFetchReply,
  type JsonFetchRequest,
  jsonFetch,
  stopScript,
  toolScript,
} from "@sage/core/testing";
import type { ProviderAdapter } from "@sage/provider";
import type { StreamChunk } from "@sage/provider/contract";
import { afterEach, describe, expect, it } from "vitest";
import { runCli } from "../src/main.ts";

let dir = "";

afterEach(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
  dir = "";
});

/** Input price per million tokens on the recorded request_header. */
const INPUT_PER_MTOK = 1;
/** Cache-read price per million tokens on the recorded request_header. */
const CACHE_READ_PER_MTOK = 0.3;
/**
 * Cache-write price per million tokens; equal to the input price, so the
 * write rate an expand pays is 1.
 */
const CACHE_WRITE_PER_MTOK = 1;
/** The Jev input price the budget gate converts with, per million tokens. */
const JEV_USD_PER_MTOK = 0.042;

/** The recorded session's tool calls, in order; `a.ts` is read twice. */
const CALLS = [
  { id: "read0", tool: "read", input: { path: "a.ts" } },
  { id: "grep1", tool: "grep", input: { pattern: "helper", path: "a.ts" } },
  { id: "read2", tool: "read", input: { path: "b.txt" } },
  {
    id: "grep3",
    tool: "grep",
    input: { pattern: "plain notes", path: "c.txt" },
  },
  { id: "read4", tool: "read", input: { path: "d.txt" } },
  { id: "read5", tool: "read", input: { path: "a.ts" } },
  { id: "bash6", tool: "bash", input: { command: "seq 1 1000" } },
] as const;
/** Calls Jev triages (grep and a runner bash); the reads get a probe that decides nothing. */
const TRIAGED = CALLS.filter((call) => call.tool !== "read").length;
/**
 * The chunk id of `grep1` in a compiler session: the Log seq of the
 * assistant message that made the call. The session expands it, so it
 * is the triaged chunk labeled needed.
 */
const EXPANDED = "seq:11#grep1";

function sourceFile(): string {
  const lines: string[] = [];
  for (let index = 0; index < 40; index += 1) {
    lines.push(`export function helper${index}(value: number) {`);
    lines.push(`  return value + ${index};`);
    lines.push("}");
  }
  return `${lines.join("\n")}\n`;
}

function proseFile(name: string): string {
  return `${Array.from({ length: 60 }, (_, index) => `${name} line ${index} of plain notes`).join("\n")}\n`;
}

/** One model call that says `prose`, then calls `tool` with `input`. */
function callWithProse(
  call: (typeof CALLS)[number],
  prose: string,
): StreamChunk[] {
  return [
    { type: "block_start", index: 0, blockType: "text" },
    { type: "text_delta", index: 0, text: prose },
    { type: "block_end", index: 0 },
    {
      type: "block_start",
      index: 1,
      blockType: "tool_use",
      id: call.id,
      name: call.tool,
    },
    {
      type: "tool_input_delta",
      index: 1,
      partialJson: JSON.stringify(call.input),
    },
    { type: "block_end", index: 1 },
    {
      type: "usage",
      usage: {
        inputOther: 1,
        inputCacheRead: 0,
        inputCacheCreation: 0,
        output: 1,
      },
    },
    { type: "finish", kind: "tool_use" },
  ];
}

type Answers = {
  failure: number;
  locate: number;
  target: number;
  keep: number;
};

/** A Jev reply that answers every question in the request body. */
function reply(
  request: JsonFetchRequest,
  answers: Answers,
  inputTokens = 100,
): JsonFetchReply {
  const body = request.body;
  const questions =
    isRecord(body) && isRecord(body.questions)
      ? Object.keys(body.questions)
      : [];
  const fixed: Record<string, number> = {
    reports_failure: answers.failure,
    locates_next_step: answers.locate,
    shows_target_code: answers.target,
  };
  const out: Record<string, { noul: number }> = {};
  for (const key of questions) {
    out[key] = { noul: fixed[key] ?? answers.keep };
  }
  return {
    json: {
      answers: out,
      usage: { input_tokens: inputTokens, output_tokens: 1 },
    },
  };
}

const KEEP_ALL: Answers = {
  failure: 0.9,
  locate: 0.1,
  target: 0.9,
  keep: 0.1,
};
const DROP_ALL: Answers = {
  failure: 0.05,
  locate: 0.05,
  target: 0.1,
  keep: 0.1,
};

/**
 * Runs a real `sage -p` session, by default with entry-mode jev scoring,
 * so the Log and the online request bodies both come from the production
 * path. Reuses the current temp directory, so two calls leave two
 * sessions side by side. A compiler session expands `EXPANDED` after
 * the calls. `passthrough` records a passthrough session without jev or
 * expand; `editAfter` adds edits of `b.txt` and `c.txt` after the calls.
 */
async function recordSession(
  opts: { passthrough?: boolean; editAfter?: boolean } = {},
): Promise<{
  file: string;
  online: JsonFetchRequest[];
}> {
  if (!dir) {
    dir = mkdtempSync(path.join(tmpdir(), "sage-calibrate-jev-"));
    writeFileSync(path.join(dir, "a.ts"), sourceFile());
    for (const name of ["b.txt", "c.txt", "d.txt", "e.txt"]) {
      writeFileSync(path.join(dir, name), proseFile(name));
    }
  }
  const scripts = CALLS.map((call, index) =>
    callWithProse(call, `Next I look for helper${index}.`),
  );
  const expand = toolScript(
    "expand0",
    "expand",
    JSON.stringify({ id: EXPANDED }),
  );
  // Ledger updates before the first read and midway, so requests carry
  // both a first ledger and a changed one.
  const ledger = (id: string, goal: string) =>
    toolScript(
      id,
      "update_ledger",
      JSON.stringify({ goal, plan: ["read files"] }),
    );
  // b.txt was read; c.txt was only grepped, so its line came from grep3.
  const edits = ["b.txt", "c.txt"].map((name, index) =>
    toolScript(
      `edit${index}`,
      "edit",
      JSON.stringify({
        path: name,
        old_string: `${name} line 0 of plain notes`,
        new_string: `${name} line 0 rewritten`,
      }),
    ),
  );
  const faux = createFaux([
    ledger("ledger0", "find the helpers"),
    ...scripts.slice(0, 3),
    ledger("ledger1", "compare the notes"),
    ...scripts.slice(3),
    ...(opts.passthrough ? [] : [expand]),
    ...(opts.editAfter ? edits : []),
    stopScript("done"),
  ]);
  const adapter: ProviderAdapter = {
    ...faux.adapter,
    prepare: () => ({
      ...faux.prepared,
      contextWindow: 200_000,
      pricing: {
        inputPerMTok: INPUT_PER_MTOK,
        outputPerMTok: 15,
        cacheReadPerMTok: CACHE_READ_PER_MTOK,
        cacheWritePerMTok: CACHE_WRITE_PER_MTOK,
        source: "test",
      },
    }),
  };
  const online = jsonFetch((request) => reply(request, KEEP_ALL));
  const sessions = path.join(dir, ".sage", "sessions");
  const before = new Set(existsSync(sessions) ? readdirSync(sessions) : []);
  const mode = opts.passthrough
    ? ["--passthrough"]
    : ["--scorer", "jev", "--scorer-apply", "entry"];
  const code = await runCli(["--print", "find the helpers", ...mode], {
    cwd: dir,
    env: { ANTHROPIC_API_KEY: "test-key", JEV_API_KEY: "x" },
    stdout: { write() {} },
    stderr: { write() {} },
    adapters: { anthropic: adapter },
    jevFetch: online.fetchImpl,
  });
  expect(code).toBe(0);
  const [name] = readdirSync(sessions).filter(
    (entry) => entry.endsWith(".jsonl") && !before.has(entry),
  );
  if (!name) throw new Error("no session written");
  return { file: path.join(sessions, name), online: online.requests() };
}

async function calibrate(
  args: string[],
  respond: (request: JsonFetchRequest) => JsonFetchReply,
  env: NodeJS.ProcessEnv = { JEV_API_KEY: "x" },
): Promise<{
  code: number;
  stdout: string;
  stderr: string;
  requests: JsonFetchRequest[];
}> {
  const fake = jsonFetch((request) => respond(request));
  let stdout = "";
  let stderr = "";
  const code = await runCli(["calibrate-jev", ...args], {
    cwd: dir,
    env,
    stdout: {
      write(chunk: string) {
        stdout += chunk;
      },
    },
    stderr: {
      write(chunk: string) {
        stderr += chunk;
      },
    },
    jevFetch: fake.fetchImpl,
  });
  return { code, stdout, stderr, requests: fake.requests() };
}

function readJson(file: string): Record<string, unknown> {
  const parsed: unknown = JSON.parse(readFileSync(file, "utf8"));
  if (!isRecord(parsed)) throw new Error(`${file} is not an object`);
  return parsed;
}

function bodiesByChunk(
  requests: readonly JsonFetchRequest[],
): Map<string, unknown> {
  const out = new Map<string, unknown>();
  for (const request of requests) {
    const body = request.body;
    if (
      !isRecord(body) ||
      !isRecord(body.state) ||
      !isRecord(body.state.chunk)
    ) {
      throw new Error("request without state.chunk");
    }
    out.set(String(body.state.chunk.id), body);
  }
  return out;
}

function chunkRows(sweep: Record<string, unknown>): Record<string, unknown>[] {
  const chunks = sweep.chunks;
  if (!Array.isArray(chunks)) throw new Error("sweep.json has no chunks");
  return chunks.filter(isRecord);
}

function rowsOf(sweep: Record<string, unknown>): Record<string, unknown>[] {
  const rows = sweep.rows;
  if (!Array.isArray(rows)) throw new Error("sweep.json has no rows");
  return rows.filter(isRecord);
}

describe("sage calibrate-jev", () => {
  it("sends Jev the online entry scorer's bodies for triaged chunks and a shows_target_code probe for each read", async () => {
    const session = await recordSession();
    const out = path.join(dir, "calibration");
    const run = await calibrate([session.file, "--out", out], (request) =>
      reply(request, KEEP_ALL),
    );
    expect(run.stderr).not.toContain("Error");
    expect(run.code).toBe(0);
    const online = bodiesByChunk(session.online);
    expect([...online.keys()].map((id) => id.split("#")[1])).toEqual([
      "grep1",
      "grep3",
      "bash6",
    ]);
    expect([...online.keys()]).toContain(EXPANDED);
    const offline = bodiesByChunk(run.requests);
    expect(offline.size).toBe(CALLS.length);
    for (const [id, body] of online) expect(offline.get(id)).toEqual(body);
    const probes = [...offline]
      .filter(([id]) => !online.has(id))
      .map(([, body]) => (isRecord(body) ? body.questions : undefined));
    expect(probes).toHaveLength(CALLS.length - TRIAGED);
    for (const questions of probes) {
      expect(Object.keys(isRecord(questions) ? questions : {})).toEqual([
        "shows_target_code",
      ]);
    }
    for (const request of run.requests) {
      expect(request.url).toBe("https://api.typesafe.ai/v1/systemone");
      expect(request.headers.Authorization).toBe("Bearer x");
    }
  });

  it("labels touch on a later re-read or expand, and content on an expand or a later pattern the chunk's output introduced", async () => {
    const session = await recordSession();
    const out = path.join(dir, "calibration");
    const run = await calibrate([session.file, "--out", out], (request) =>
      reply(request, KEEP_ALL),
    );
    expect(run.code).toBe(0);
    const labels = chunkRows(readJson(path.join(out, "sweep.json"))).map(
      (row) => [String(row.chunkId).split("#")[1], row.touch, row.content],
    );
    // read0 is only touched: read5 re-reads a.ts, whose path read0's own
    // input named. read2 is used: b.txt is the first text to show "plain
    // notes", the pattern grep3 searches for.
    expect(labels).toEqual([
      ["read0", true, false],
      ["grep1", true, true],
      ["read2", false, true],
      ["grep3", false, false],
      ["read4", false, false],
      ["read5", false, false],
      ["bash6", false, false],
    ]);
  });

  it("lists every answered chunk in sweep.json with its tool, bash kind, question set, and answer probabilities", async () => {
    const session = await recordSession();
    const out = path.join(dir, "calibration");
    const run = await calibrate([session.file, "--out", out], (request) =>
      reply(request, KEEP_ALL),
    );
    expect(run.code).toBe(0);
    const rows = chunkRows(readJson(path.join(out, "sweep.json")));
    const grep = rows.find((row) => row.chunkId === EXPANDED);
    expect(grep).toMatchObject({
      tool: "grep",
      bashKind: null,
      ledgerEmpty: false,
      turn: 4,
      questions: "triage",
    });
    expect(rows.find((row) => row.tool === "bash")).toMatchObject({
      bashKind: "runner",
      questions: "triage",
    });
    if (!isRecord(grep?.answers)) throw new Error("grep row has no answers");
    expect(grep.answers.reports_failure).toBe(0.9);
    expect(grep.answers.locates_next_step).toBe(0.1);
    expect(Array.isArray(grep.answers.keep)).toBe(true);
    expect(grep.answers.keep).not.toHaveLength(0);
    for (const p of grep.answers.keep as unknown[]) expect(p).toBe(0.1);
    const reads = rows.filter((row) => row.tool === "read");
    expect(reads).toHaveLength(CALLS.length - TRIAGED);
    for (const row of reads) {
      expect(row).toMatchObject({
        bashKind: null,
        questions: "read-probe",
        answers: { shows_target_code: 0.9 },
      });
    }
  });

  it("answers a rerun from the cache without sending or billing", async () => {
    const session = await recordSession();
    const out = path.join(dir, "calibration");
    const first = await calibrate([session.file, "--out", out], (request) =>
      reply(request, KEEP_ALL),
    );
    expect(first.code).toBe(0);
    expect(readdirSync(path.join(out, "cache"))).toHaveLength(CALLS.length);
    const spent = readJson(path.join(out, "spend.json"));
    expect(spent.inputTokens).toBe(100 * CALLS.length);

    const second = await calibrate([session.file, "--out", out], () => {
      throw new Error("a cached request was sent");
    });
    expect(second.code).toBe(0);
    expect(second.requests).toEqual([]);
    expect(readJson(path.join(out, "spend.json"))).toEqual(spent);
    const summary: unknown = JSON.parse(second.stdout);
    expect(summary).toMatchObject({
      chunks: CALLS.length,
      cached: CALLS.length,
      sent: 0,
    });
  });

  it("prints the estimate on --dry-run without sending, writing spend, or needing a key", async () => {
    const session = await recordSession();
    const out = path.join(dir, "calibration");
    const run = await calibrate(
      [session.file, "--out", out, "--dry-run"],
      () => {
        throw new Error("dry run sent a request");
      },
      {},
    );
    expect(run.code).toBe(0);
    expect(run.requests).toEqual([]);
    expect(existsSync(path.join(out, "spend.json"))).toBe(false);
    const summary: unknown = JSON.parse(run.stdout);
    if (!isRecord(summary) || !isRecord(summary.estimate)) {
      throw new Error("dry run printed no estimate");
    }
    expect(summary.dryRun).toBe(true);
    expect(summary.pending).toBe(CALLS.length);
    const tokens = Number(summary.estimate.inputTokens);
    expect(tokens).toBeGreaterThan(0);
    expect(summary.estimate.usd).toBeCloseTo(
      (tokens * JEV_USD_PER_MTOK) / 1e6,
      12,
    );
  });

  it("refuses with exit 2 when the estimate exceeds --max-usd", async () => {
    const session = await recordSession();
    const out = path.join(dir, "calibration");
    const run = await calibrate(
      [session.file, "--out", out, "--max-usd", "0"],
      () => {
        throw new Error("an over-budget run sent a request");
      },
    );
    expect(run.code).toBe(2);
    expect(run.requests).toEqual([]);
    expect(run.stderr).toContain("--max-usd");
    expect(existsSync(path.join(out, "spend.json"))).toBe(false);
  });

  it("counts spend already in spend.json against --max-usd", async () => {
    const session = await recordSession();
    const out = path.join(dir, "calibration");
    mkdirSync(out, { recursive: true });
    // 30M tokens is 1.26 USD, past the default 1 USD cap before any request.
    const prior = {
      requests: 1,
      inputTokens: 30_000_000,
      outputTokens: 0,
      usd: 1.26,
    };
    writeFileSync(path.join(out, "spend.json"), JSON.stringify(prior));
    const run = await calibrate([session.file, "--out", out], () => {
      throw new Error("a run over its cumulative cap sent a request");
    });
    expect(run.code).toBe(2);
    expect(run.requests).toEqual([]);
    expect(run.stderr).toContain("$1.2600");
  });

  it("exits 1 and names both key variables when uncached requests need a key", async () => {
    const session = await recordSession();
    const out = path.join(dir, "calibration");
    const run = await calibrate(
      [session.file, "--out", out],
      () => {
        throw new Error("a keyless run sent a request");
      },
      {},
    );
    expect(run.code).toBe(1);
    expect(run.requests).toEqual([]);
    expect(run.stderr).toContain("JEV_API_KEY");
    expect(run.stderr).toContain("TYPESAFE_API_KEY");
  });

  it("stops sending once actual spend passes --max-usd and still writes what it got", async () => {
    const session = await recordSession();
    const out = path.join(dir, "calibration");
    // One reply alone costs 0.042 USD, past the 0.01 cap; the estimate is far below it.
    const run = await calibrate(
      [session.file, "--out", out, "--max-usd", "0.01"],
      (request) => reply(request, KEEP_ALL, 1_000_000),
    );
    expect(run.code).toBe(2);
    // Four requests are in flight when the first reply lands; no fifth starts.
    expect(run.requests).toHaveLength(4);
    expect(readdirSync(path.join(out, "cache"))).toHaveLength(4);
    const spent = readJson(path.join(out, "spend.json"));
    expect(spent.inputTokens).toBe(4_000_000);
    expect(spent.usd).toBeCloseTo(4 * JEV_USD_PER_MTOK, 12);
    const sweep = readJson(path.join(out, "sweep.json"));
    expect(chunkRows(sweep)).toHaveLength(4);
  });

  it("writes one sweep row per threshold combination over the triaged chunks with tiers, labels, and input-equivalent cost", async () => {
    const session = await recordSession();
    const out = path.join(dir, "calibration");
    const run = await calibrate(
      [
        session.file,
        "--out",
        out,
        "--t-full",
        "0.4",
        "--t-low",
        "0.05,0.2",
        "--t-keep",
        "0.3,0.5",
      ],
      (request) => reply(request, DROP_ALL),
    );
    expect(run.code).toBe(0);
    const sweep = readJson(path.join(out, "sweep.json"));
    const chunks = chunkRows(sweep);
    expect(new Set(chunks.map((row) => row.session))).toEqual(
      new Set([path.basename(session.file)]),
    );
    const triaged = chunks.filter((row) => row.questions === "triage");
    expect(triaged).toHaveLength(TRIAGED);
    const needed = triaged.find((row) => row.content === true);
    if (!needed) throw new Error("no needed triaged chunk");
    const rows = rowsOf(sweep);
    expect(rows.map((row) => row.thresholds)).toEqual([
      { full: 0.4, low: 0.05, keep: 0.3 },
      { full: 0.4, low: 0.05, keep: 0.5 },
      { full: 0.4, low: 0.2, keep: 0.3 },
      { full: 0.4, low: 0.2, keep: 0.5 },
    ]);
    const [keeping, , dropping] = rows;
    expect(dropping).toMatchObject({
      tiers: { full: 0, summary: 0, none: TRIAGED },
      omitted: TRIAGED,
      needed: 1,
      omittedNeeded: 1,
      missRate: 1,
      precision: 2 / 3,
      recall: 1,
    });
    // An expand pays the write rate: max(cacheWrite, input) / input = 1.
    expect(dropping?.cost).toBeCloseTo(Number(needed.tokens), 9);
    // Every answer clears low 0.05 and no keep line reaches keep 0.3, so
    // nothing drops or summarizes: every triaged chunk stays full.
    expect(keeping).toMatchObject({
      tiers: { full: TRIAGED, summary: 0, none: 0 },
      omitted: 0,
      needed: 1,
      omittedNeeded: 0,
      missRate: 0,
      precision: null,
      recall: 0,
    });
    const readRate = CACHE_READ_PER_MTOK / INPUT_PER_MTOK;
    const keptCost = triaged
      .filter((row) => row.content === false)
      .reduce(
        (total, row) =>
          total + Number(row.tokens) * readRate * Number(row.remainingTurns),
        0,
      );
    expect(keeping?.cost).toBeCloseTo(keptCost, 9);
    expect(Number(dropping?.cost)).toBeLessThan(keptCost);
    expect(run.stdout.trim().split("\n")).toHaveLength(1);
    const summary: unknown = JSON.parse(run.stdout);
    // The two dropping rows tie on cost; the one nearer the defaults wins,
    // not the grid's first such row.
    expect(summary).toMatchObject({
      unpriced: 0,
      recommended: { thresholds: { full: 0.4, low: 0.2, keep: 0.5 } },
    });
  });

  it("counts chunks without recorded pricing as unpriced and prices them at the default rates", async () => {
    const session = await recordSession();
    const out = path.join(dir, "calibration");
    const unpriced = readFileSync(session.file, "utf8")
      .split("\n")
      .map((line) => {
        if (!line) return line;
        const entry: unknown = JSON.parse(line);
        if (!isRecord(entry) || entry.kind !== "request_header") return line;
        if (!isRecord(entry.payload)) return line;
        return JSON.stringify({
          ...entry,
          payload: { ...entry.payload, pricing: null },
        });
      })
      .join("\n");
    writeFileSync(session.file, unpriced);
    const run = await calibrate(
      [
        session.file,
        "--out",
        out,
        "--t-full",
        "0.4",
        "--t-low",
        "0.2",
        "--t-keep",
        "0.5",
      ],
      (request) => reply(request, DROP_ALL),
    );
    expect(run.code).toBe(0);
    const sweep = readJson(path.join(out, "sweep.json"));
    const needed = chunkRows(sweep).find(
      (row) => row.questions === "triage" && row.content === true,
    );
    const [row] = rowsOf(sweep);
    // Without pricing an expand pays the default write rate, 1.25.
    expect(row?.cost).toBeCloseTo(Number(needed?.tokens) * 1.25, 9);
    expect(JSON.parse(run.stdout)).toMatchObject({ unpriced: TRIAGED });
  });

  it("labels a grep used in a passthrough session when a later edit's old_string came from its output", async () => {
    const session = await recordSession({ passthrough: true, editAfter: true });
    const out = path.join(dir, "calibration");
    const run = await calibrate([session.file, "--out", out], (request) =>
      reply(request, KEEP_ALL),
    );
    expect(run.code).toBe(0);
    const rows = chunkRows(readJson(path.join(out, "sweep.json")));
    const labels = rows.map((row) => [
      String(row.chunkId).split("#")[1],
      row.touch,
      row.content,
    ]);
    // Without expand, touch never marks a grep; content marks grep3, whose
    // output is where the edited c.txt line first appeared.
    expect(labels).toEqual([
      ["read0", true, false],
      ["grep1", false, false],
      ["read2", true, true],
      ["grep3", false, true],
      ["read4", false, false],
      ["read5", false, false],
      ["bash6", false, false],
    ]);
    // Passthrough renders no claims, so no request carries a ledger.
    const ledgers = run.requests.map((request) => {
      const body = request.body;
      return isRecord(body) && isRecord(body.state) ? body.state.ledger : null;
    });
    expect(ledgers).toEqual(CALLS.map(() => ""));
    for (const row of rows) expect(row.ledgerEmpty).toBe(true);
  });

  it("sweeps on the content label by default and on touch with --label touch", async () => {
    const session = await recordSession({ passthrough: true, editAfter: true });
    const grid = ["--t-full", "0.4", "--t-low", "0.2", "--t-keep", "0.5"];
    const byContent = path.join(dir, "by-content");
    const byTouch = path.join(dir, "by-touch");
    const first = await calibrate(
      [session.file, "--out", byContent, ...grid],
      (request) => reply(request, DROP_ALL),
    );
    const second = await calibrate(
      [session.file, "--out", byTouch, ...grid, "--label", "touch"],
      (request) => reply(request, DROP_ALL),
    );
    expect(first.code).toBe(0);
    expect(second.code).toBe(0);
    const content = readJson(path.join(byContent, "sweep.json"));
    const touch = readJson(path.join(byTouch, "sweep.json"));
    expect(content.label).toBe("content");
    expect(touch.label).toBe("touch");
    // Of the triaged grep1, grep3, and bash6, only grep3 is used, and
    // nothing is touched.
    expect(rowsOf(content)[0]).toMatchObject({ needed: 1, omittedNeeded: 1 });
    expect(rowsOf(touch)[0]).toMatchObject({ needed: 0, omittedNeeded: 0 });
    expect(JSON.parse(first.stdout)).toMatchObject({ label: "content" });
    expect(JSON.parse(second.stdout)).toMatchObject({ label: "touch" });
  });

  it("rejects a --label other than touch or content with the usage", async () => {
    const session = await recordSession();
    const run = await calibrate(
      [session.file, "--out", path.join(dir, "bad"), "--label", "needed"],
      () => {
        throw new Error("a bad --label sent a request");
      },
    );
    expect(run.code).toBe(1);
    expect(run.stderr).toContain("--label touch|content");
  });

  it("calibrates several sessions at once, tagging each chunk with its session and sending a shared body once", async () => {
    const first = await recordSession();
    const second = await recordSession();
    const out = path.join(dir, "calibration");
    const run = await calibrate(
      [first.file, second.file, "--out", out],
      (request) => reply(request, KEEP_ALL),
    );
    expect(run.code).toBe(0);
    const chunks = chunkRows(readJson(path.join(out, "sweep.json")));
    expect(chunks.map((row) => row.session)).toEqual([
      ...CALLS.map(() => path.basename(first.file)),
      ...CALLS.map(() => path.basename(second.file)),
    ]);
    // Same files, same task: the two sessions ask byte-identical questions.
    expect(run.requests).toHaveLength(CALLS.length);
    expect(JSON.parse(run.stdout)).toMatchObject({
      sessions: 2,
      chunks: 2 * CALLS.length,
      sent: CALLS.length,
    });
  });

  it("keeps sending after a failed request and exits 1 with the failure counted", async () => {
    const session = await recordSession();
    const out = path.join(dir, "calibration");
    let calls = 0;
    const run = await calibrate([session.file, "--out", out], (request) => {
      calls += 1;
      return calls === 1
        ? { status: 500, text: "boom" }
        : reply(request, KEEP_ALL);
    });
    expect(run.code).toBe(1);
    expect(run.requests).toHaveLength(CALLS.length);
    expect(readdirSync(path.join(out, "cache"))).toHaveLength(CALLS.length - 1);
    expect(JSON.parse(run.stdout)).toMatchObject({
      sent: CALLS.length,
      failed: 1,
      unsent: 0,
      stopped: null,
    });
    expect(chunkRows(readJson(path.join(out, "sweep.json")))).toHaveLength(
      CALLS.length - 1,
    );
  });

  it("stops sending once a request stays rate-limited through every retry", async () => {
    const session = await recordSession();
    const out = path.join(dir, "calibration");
    const run = await calibrate([session.file, "--out", out], () => ({
      status: 429,
      text: "slow down",
      headers: { "retry-after": "0" },
    }));
    expect(run.code).toBe(1);
    // Four requests are in flight when the breaker opens; each makes its
    // first attempt and three retries, and the last two are never sent.
    const bodies = new Set(
      run.requests.map((request) => JSON.stringify(request.body)),
    );
    expect(bodies.size).toBe(4);
    expect(run.requests).toHaveLength(16);
    expect(JSON.parse(run.stdout)).toMatchObject({
      failed: 4,
      unsent: CALLS.length - 4,
      stopped: { kind: "rate-limited", fallback: "http:429" },
    });
  });
});
