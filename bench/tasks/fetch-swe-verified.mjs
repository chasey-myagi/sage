/**
 * Pull the first 50 SWE-bench Verified rows into a sage bench task file.
 *
 * Zero dependencies. The Hugging Face datasets-server rows API is the only
 * network call. `--fixture <file>` reads that JSON instead, so tests never
 * hit the network. FAIL_TO_PASS becomes `python -m pytest <tests>`.
 * timeoutSec is 1800 for every row. This script does not run the benchmark.
 *
 * Not a Log entry and not model-visible. The task file is an operator input.
 */

import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

/** `timeoutSec` written into every task, in seconds: the runner applies it to setup, the agent, and the test command, each. */
const TIMEOUT_SEC = 1800;
/** Default number of tasks taken from the dataset when `--limit` is not given. */
const LIMIT = 50;

const DATASET = "princeton-nlp/SWE-bench_Verified";
const ROWS_URL = "https://datasets-server.huggingface.co/rows";

function tasksFromDataset(payload, limit = LIMIT) {
  return extractRows(payload)
    .slice(0, limit)
    .map((row) => toTask(row));
}

function extractRows(payload) {
  if (Array.isArray(payload)) return payload;
  if (payload && Array.isArray(payload.rows)) {
    return payload.rows.map((item) =>
      item && typeof item === "object" && "row" in item && item.row
        ? item.row
        : item,
    );
  }
  throw new Error("dataset payload has no rows");
}

function toTask(row) {
  if (!row || typeof row !== "object") {
    throw new Error("dataset row is not an object");
  }
  const id = required(row, "instance_id");
  const tests = failToPass(row.FAIL_TO_PASS, id);
  return {
    id,
    repo: required(row, "repo"),
    baseCommit: required(row, "base_commit"),
    problem: required(row, "problem_statement"),
    testCommand: `python -m pytest ${tests.map(quoteToken).join(" ")}`,
    timeoutSec: TIMEOUT_SEC,
  };
}

function failToPass(value, id) {
  let parsed = value;
  if (typeof value === "string") {
    try {
      parsed = JSON.parse(value);
    } catch {
      throw new Error(`FAIL_TO_PASS for ${id} is not JSON`);
    }
  }
  if (
    !Array.isArray(parsed) ||
    parsed.length === 0 ||
    parsed.some((item) => typeof item !== "string" || item.length === 0)
  ) {
    throw new Error(`FAIL_TO_PASS for ${id} must be a non-empty list of tests`);
  }
  return parsed;
}

function required(row, key) {
  const value = row[key];
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`dataset row missing ${key}`);
  }
  return value;
}

function quoteToken(token) {
  if (/^[A-Za-z0-9_./:+-]+$/.test(token)) return token;
  return `'${token.replaceAll("'", "'\\''")}'`;
}

async function fetchDataset(limit) {
  const url = new URL(ROWS_URL);
  url.searchParams.set("dataset", DATASET);
  url.searchParams.set("config", "default");
  url.searchParams.set("split", "test");
  url.searchParams.set("offset", "0");
  url.searchParams.set("length", String(limit));
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`dataset fetch failed: ${response.status}`);
  }
  return response.json();
}

function parseArgs(argv) {
  let fixture;
  let out = path.join(
    path.dirname(fileURLToPath(import.meta.url)),
    "swe-verified-mini-50.json",
  );
  let limit = LIMIT;
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index] ?? "";
    if (arg === "--fixture") {
      fixture = argv[index + 1];
      index += 1;
      continue;
    }
    if (arg === "--out") {
      out = argv[index + 1];
      index += 1;
      continue;
    }
    if (arg === "--limit") {
      limit = Number(argv[index + 1]);
      index += 1;
      continue;
    }
    throw new Error(
      "usage: node bench/tasks/fetch-swe-verified.mjs [--fixture <file>] [--out <file>] [--limit 50]",
    );
  }
  if (!out || !Number.isInteger(limit) || limit <= 0) {
    throw new Error("fetch-swe-verified: invalid --out or --limit");
  }
  return { fixture, out, limit };
}

export async function main(argv = process.argv.slice(2)) {
  const args = parseArgs(argv);
  const payload = args.fixture
    ? JSON.parse(readFileSync(args.fixture, "utf8"))
    : await fetchDataset(args.limit);
  const tasks = tasksFromDataset(payload, args.limit);
  writeFileSync(args.out, `${JSON.stringify(tasks, null, 2)}\n`);
  return tasks.length;
}

const entry = process.argv[1];
if (entry && import.meta.url === pathToFileURL(path.resolve(entry)).href) {
  main().catch((error) => {
    const message = error instanceof Error ? error.message : String(error);
    process.stderr.write(`${message}\n`);
    process.exit(1);
  });
}
