/**
 * Build the TS/JS bench task file from Multi-SWE-bench (ByteDance-Seed).
 *
 * Zero dependencies. The datasets-server rows API fails on this dataset
 * (schema cast errors), so this script reads the per-repo JSONL files through
 * `huggingface.co/datasets/<ds>/resolve/main/<lang>/<org>__<repo>_dataset.jsonl`.
 * `--fixture <dir>` reads `<dir>/<org>__<repo>_dataset.jsonl` instead, so tests
 * never hit the network.
 *
 * Only the instances in SELECTION are emitted. Each one was verified on a real
 * clone: after setup, applying the test patch and running the test command
 * fails at baseCommit and passes after the gold fix patch. setup leaves
 * `git status` empty.
 *
 * problem      = title + body of every resolved issue (no PR text, no diff).
 * setupCommand = install deps after checkout, before the agent starts.
 * testPatch    = the dataset test_patch, applied by the runner after the agent
 *               exits. The agent never sees it.
 * testCommand  = the test runner invocation for the spec files in that patch.
 */

import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const TIMEOUT_SEC = 1800;

const DATASET = "ByteDance-Seed/Multi-SWE-bench";
const RESOLVE_URL = `https://huggingface.co/datasets/${DATASET}/resolve/main`;

const NPM_CI = "PUPPETEER_SKIP_DOWNLOAD=1 npm ci --no-audit --no-fund";
const PNPM_FROZEN =
  "PUPPETEER_SKIP_DOWNLOAD=1 corepack pnpm install --frozen-lockfile";

// Per-repo install and runner. `run(files)` gets the spec files of the test patch.
const REPOS = {
  "vuejs/core": {
    lang: "ts",
    setup: PNPM_FROZEN,
    run: (files) => `corepack pnpm exec vitest run ${files.join(" ")}`,
  },
  "darkreader/darkreader": {
    lang: "ts",
    setup: NPM_CI,
    run: (files) =>
      `npx --no-install jest --config=tests/jest.config.js ${files.join(" ")}`,
  },
  "iamkun/dayjs": {
    lang: "js",
    setup: NPM_CI,
    // Some date tests depend on the host zone (year-1 LMT offsets). Pin UTC.
    run: (files) => `TZ=UTC npx --no-install jest ${files.join(" ")}`,
  },
  "expressjs/express": {
    lang: "js",
    // No lockfile in the repo. --no-package-lock keeps `git status` clean.
    setup: "npm install --no-audit --no-fund --no-package-lock",
    run: (files) =>
      `npx --no-install mocha --require test/support/env --check-leaks ${files.join(" ")}`,
  },
  "axios/axios": {
    lang: "js",
    setup: NPM_CI,
    run: (files) =>
      `npx --no-install mocha --timeout 30000 --exit ${files.join(" ")}`,
  },
  "anuraghazra/github-readme-stats": {
    lang: "js",
    setup: NPM_CI,
    run: (files) =>
      `node --experimental-vm-modules node_modules/jest/bin/jest.js --ci ${files.join(" ")}`,
  },
};

export const SELECTION = [
  "vuejs__core-11813",
  "vuejs__core-10289",
  "vuejs__core-10874",
  "vuejs__core-11625",
  "darkreader__darkreader-7241",
  "darkreader__darkreader-6747",
  "iamkun__dayjs-2377",
  "expressjs__express-5555",
  "axios__axios-5661",
  "anuraghazra__github-readme-stats-2228",
];

const SPEC_FILE =
  /\.(spec|test|tests)\.[cm]?[jt]sx?$|^test\/[^/]+\.js$|^test\/unit\/.+\.js$/;

export function taskFromRow(row) {
  const repo = `${row.org}/${row.repo}`;
  const conf = REPOS[repo];
  if (!conf) throw new Error(`no repo config for ${repo}`);
  const touched = patchFiles(row.test_patch);
  const specs = touched.filter(
    (f) => SPEC_FILE.test(f) && !f.includes("/support/"),
  );
  if (specs.length === 0) {
    throw new Error(`${row.instance_id}: test patch has no runnable spec file`);
  }
  return {
    id: row.instance_id,
    repo,
    baseCommit: row.base.sha,
    problem: problemText(row),
    setupCommand: conf.setup,
    testPatch: row.test_patch,
    testCommand: conf.run(specs),
    timeoutSec: TIMEOUT_SEC,
  };
}

function problemText(row) {
  const issues = row.resolved_issues ?? [];
  if (issues.length === 0)
    throw new Error(`${row.instance_id}: no resolved issue`);
  return issues
    .map((issue) => `${issue.title}\n\n${(issue.body ?? "").trim()}`.trim())
    .join("\n\n---\n\n");
}

function patchFiles(patch) {
  return [...patch.matchAll(/^diff --git a\/\S+ b\/(\S+)$/gm)].map((m) => m[1]);
}

function jsonlName(repo) {
  return `${repo.replace("/", "__")}_dataset.jsonl`;
}

async function loadRows(repo, fixtureDir) {
  const name = jsonlName(repo);
  let text;
  if (fixtureDir) {
    text = readFileSync(path.join(fixtureDir, name), "utf8");
  } else {
    const url = `${RESOLVE_URL}/${REPOS[repo].lang}/${name}`;
    const response = await fetch(url);
    if (!response.ok)
      throw new Error(`fetch ${url} failed: ${response.status}`);
    text = await response.text();
  }
  return text
    .split("\n")
    .filter((line) => line.trim().length > 0)
    .map((line) => JSON.parse(line));
}

function repoOf(instanceId) {
  const [owner, rest] = instanceId.split("__");
  return `${owner}/${rest.replace(/-\d+$/, "")}`;
}

export async function buildTasks({ fixtureDir, selection = SELECTION } = {}) {
  const byRepo = new Map();
  for (const id of selection) {
    const repo = repoOf(id);
    if (!byRepo.has(repo)) byRepo.set(repo, []);
    byRepo.get(repo).push(id);
  }
  const found = new Map();
  for (const [repo, ids] of byRepo) {
    const rows = await loadRows(repo, fixtureDir);
    for (const row of rows) {
      if (ids.includes(row.instance_id))
        found.set(row.instance_id, taskFromRow(row));
    }
  }
  return selection.map((id) => {
    const task = found.get(id);
    if (!task) throw new Error(`instance ${id} not in dataset`);
    return task;
  });
}

function parseArgs(argv) {
  let fixtureDir;
  let only;
  let out = path.join(
    path.dirname(fileURLToPath(import.meta.url)),
    "ts-mini.json",
  );
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--fixture") fixtureDir = argv[++i];
    else if (arg === "--out") out = argv[++i];
    else if (arg === "--only") only = argv[++i].split(",");
    else {
      throw new Error(
        "usage: node bench/tasks/fetch-multi-swe-ts.mjs [--fixture <dir>] [--out <file>] [--only id1,id2]",
      );
    }
  }
  return { fixtureDir, out, only };
}

export async function main(argv = process.argv.slice(2)) {
  const args = parseArgs(argv);
  const tasks = await buildTasks({
    fixtureDir: args.fixtureDir,
    selection: args.only ?? SELECTION,
  });
  writeFileSync(args.out, `${JSON.stringify(tasks, null, 2)}\n`);
  return tasks.length;
}

const entry = process.argv[1];
if (entry && import.meta.url === pathToFileURL(path.resolve(entry)).href) {
  main().catch((error) => {
    process.stderr.write(
      `${error instanceof Error ? error.message : String(error)}\n`,
    );
    process.exit(1);
  });
}
