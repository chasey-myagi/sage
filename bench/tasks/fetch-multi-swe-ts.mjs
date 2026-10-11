/**
 * Build the TS/JS bench task files from ByteDance-Seed Multi-SWE-bench and
 * Multi-SWE-RL.
 *
 * Zero dependencies. Each repo in REPOS declares its dataset and path; rows are
 * read from `huggingface.co/datasets/<dataset>/resolve/main/<path>/<org>__<repo>_dataset.jsonl`
 * (the datasets-server rows API fails on these datasets with schema cast
 * errors). `--fixture <dir>` reads `<dir>/<org>__<repo>_dataset.jsonl` instead,
 * so tests never hit the network.
 *
 * `--set <name>` picks one of SETS (default `ts-mini`) and its default output
 * file. Only the ids of that set are emitted, in set order. Each one was
 * verified on a real clone: after setup `git status` is empty, the test patch
 * fails at baseCommit and passes after the gold fix patch. Multi-SWE-RL is an
 * unreviewed RL training set, so nothing enters a set without that check.
 *
 * id           = `instance_id`, or `<org>__<repo>-<number>` when the row has none.
 * problem      = title + body of every resolved issue (no PR text, no diff).
 * setupCommand = install deps after checkout, before the agent starts.
 * testPatch    = the dataset test_patch, applied by the runner after the agent
 *               exits. The agent never sees it.
 * testCommand  = the test runner invocation for the spec files in that patch;
 *               it may build first when the tests import build output.
 */

import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { parseArgs } from "node:util";

/** `timeoutSec` written into every task, in seconds: the runner applies it to setup, the agent, and the test command, each. */
const TIMEOUT_SEC = 1800;

const MULTI_SWE_BENCH = "ByteDance-Seed/Multi-SWE-bench";
const MULTI_SWE_RL = "ByteDance-Seed/Multi-SWE-RL";
const RL_2024 = "data_20240601_20250331";
const RL_2025 = "data_20250401_20250631";

const NPM_CI = "PUPPETEER_SKIP_DOWNLOAD=1 npm ci --no-audit --no-fund";
// No lockfile, or one out of sync. --no-package-lock keeps `git status` clean.
const NPM_NO_LOCK = "npm install --no-audit --no-fund --no-package-lock";
const PNPM_FROZEN =
  "PUPPETEER_SKIP_DOWNLOAD=1 corepack pnpm install --frozen-lockfile";
const PNPM_9_9 = "corepack pnpm@9.9.0";
const YARN_1 = "corepack yarn@1.22.22";
const YARN_1_FLAGS = "--ignore-engines --non-interactive";

// Per-repo source, install and runner. `run(files)` gets the spec files of
// the test patch.
const REPOS = {
  "vuejs/core": {
    dataset: MULTI_SWE_BENCH,
    path: "ts",
    setup: PNPM_FROZEN,
    run: (files) => `corepack pnpm exec vitest run ${files.join(" ")}`,
  },
  "darkreader/darkreader": {
    dataset: MULTI_SWE_BENCH,
    path: "ts",
    setup: NPM_CI,
    run: (files) =>
      `npx --no-install jest --config=tests/jest.config.js ${files.join(" ")}`,
  },
  "iamkun/dayjs": {
    dataset: MULTI_SWE_BENCH,
    path: "js",
    setup: NPM_CI,
    // Some date tests depend on the host zone (year-1 LMT offsets). Pin UTC.
    run: (files) => `TZ=UTC npx --no-install jest ${files.join(" ")}`,
  },
  "expressjs/express": {
    dataset: MULTI_SWE_BENCH,
    path: "js",
    setup: NPM_NO_LOCK,
    run: (files) =>
      `npx --no-install mocha --require test/support/env --check-leaks ${files.join(" ")}`,
  },
  "axios/axios": {
    dataset: MULTI_SWE_BENCH,
    path: "js",
    setup: NPM_CI,
    run: (files) =>
      `npx --no-install mocha --timeout 30000 --exit ${files.join(" ")}`,
  },
  "anuraghazra/github-readme-stats": {
    dataset: MULTI_SWE_BENCH,
    path: "js",
    setup: NPM_CI,
    run: (files) =>
      `node --experimental-vm-modules node_modules/jest/bin/jest.js --ci ${files.join(" ")}`,
  },
  "google/zx": {
    dataset: MULTI_SWE_RL,
    path: `${RL_2024}/js`,
    // Tests import build/, so both setup and the test run build first.
    setup: `${NPM_CI} && npm run build:js`,
    run: (files) =>
      `npm run build:js >/dev/null && node --experimental-strip-types --test ${files.join(" ")}`,
  },
  "tj/commander.js": {
    dataset: MULTI_SWE_RL,
    path: `${RL_2024}/js`,
    setup: NPM_CI,
    run: (files) => `npx --no-install jest ${files.join(" ")}`,
  },
  "caolan/async": {
    dataset: MULTI_SWE_RL,
    path: `${RL_2024}/js`,
    setup: NPM_CI,
    run: (files) => `npx --no-install mocha ${files.join(" ")}`,
  },
  "reduxjs/redux": {
    dataset: MULTI_SWE_RL,
    path: `${RL_2024}/ts`,
    // Tests import the package entry (lib/), so the build runs before jest.
    setup: NPM_CI,
    run: (files) =>
      `npm run build >/dev/null 2>&1 && npx --no-install jest ${files.join(" ")}`,
  },
  "colinhacks/zod": {
    dataset: MULTI_SWE_RL,
    path: `${RL_2024}/ts`,
    // yarn.lock is out of sync with package.json; --pure-lockfile keeps it unwritten.
    setup: `PUPPETEER_SKIP_DOWNLOAD=1 ${YARN_1} install --pure-lockfile ${YARN_1_FLAGS}`,
    run: (files) =>
      `npx --no-install jest --config ./configs/ts-jest.config.json ${files.join(" ")}`,
  },
  "remix-run/react-router": {
    dataset: MULTI_SWE_RL,
    path: `${RL_2024}/ts`,
    setup: PNPM_FROZEN,
    run: (files) => `corepack pnpm exec jest ${files.join(" ")}`,
  },
  "react-hook-form/react-hook-form": {
    dataset: MULTI_SWE_RL,
    path: `${RL_2025}/ts`,
    setup: PNPM_FROZEN,
    run: (files) =>
      `corepack pnpm exec jest --config ./scripts/jest/jest.config.js ${files.join(" ")}`,
  },
};

// Per-instance setup/run that replace the repo defaults for one base commit.
const OVERRIDES = {
  // This base still uses yarn1 (yarn.lock), not pnpm.
  "remix-run__react-router-11160": {
    setup: `PUPPETEER_SKIP_DOWNLOAD=1 ${YARN_1} install --frozen-lockfile ${YARN_1_FLAGS}`,
    run: (files) => `npx --no-install jest ${files.join(" ")}`,
  },
  // No packageManager field: bare `corepack pnpm` resolves a pnpm that
  // corepack 0.34 cannot start. Pin the version matching lockfile 9.0.
  "react-hook-form__react-hook-form-12505": {
    setup: `PUPPETEER_SKIP_DOWNLOAD=1 ${PNPM_9_9} install --frozen-lockfile`,
    run: (files) =>
      `${PNPM_9_9} exec jest --config ./scripts/jest/jest.config.js ${files.join(" ")}`,
  },
  // package-lock.json misses watchify, so `npm ci` fails.
  "caolan__async-1790": { setup: NPM_NO_LOCK },
};

/** Named task sets: the verified ids, in output order, and the default output file. */
const SETS = {
  "ts-mini": {
    out: "ts-mini.json",
    ids: [
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
    ],
  },
  "ts-mini-2": {
    out: "ts-mini-2.json",
    ids: [
      "google__zx-1107",
      "tj__commander.js-1720",
      "tj__commander.js-1502",
      "reduxjs__redux-4139",
      "colinhacks__zod-3427",
      "remix-run__react-router-11786",
      "remix-run__react-router-11160",
      "caolan__async-1790",
      "caolan__async-1628",
      "react-hook-form__react-hook-form-12505",
    ],
  },
};

const DEFAULT_SET = "ts-mini";

const SPEC_FILE =
  /\.(spec|test|tests)\.[cm]?[jt]sx?$|(^|\/)[^/]+-(spec|test)\.[jt]sx?$|^tests?\/[^/]+\.js$|^test\/unit\/.+\.js$/;

function isSpecFile(file) {
  return (
    SPEC_FILE.test(file) &&
    !file.includes("/support/") &&
    !file.includes("__snapshots__") &&
    !file.startsWith("deno/")
  );
}

/** Task id of a dataset row; rows of the 2025 Multi-SWE-RL period carry no `instance_id`. */
function idOf(row) {
  return row.instance_id ?? `${row.org}__${row.repo}-${row.number}`;
}

/** Convert one dataset row into a bench task. The row's repo must be in REPOS (`buildTasks` checks this first). Throws when the row has no spec file or resolved issue. */
function taskFromRow(row) {
  const id = idOf(row);
  const repo = `${row.org}/${row.repo}`;
  const conf = { ...REPOS[repo], ...OVERRIDES[id] };
  const specs = patchFiles(row.test_patch).filter(isSpecFile);
  if (specs.length === 0) {
    throw new Error(`${id}: test patch has no runnable spec file`);
  }
  return {
    id,
    repo,
    baseCommit: row.base.sha,
    problem: problemText(id, row),
    setupCommand: conf.setup,
    testPatch: row.test_patch,
    testCommand: conf.run(specs),
    timeoutSec: TIMEOUT_SEC,
  };
}

function problemText(id, row) {
  const issues = row.resolved_issues ?? [];
  if (issues.length === 0) throw new Error(`${id}: no resolved issue`);
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
    const { dataset, path: dataPath } = REPOS[repo];
    const url = `https://huggingface.co/datasets/${dataset}/resolve/main/${dataPath}/${name}`;
    const response = await fetch(url);
    if (!response.ok)
      throw new Error(`fetch ${url} failed: ${response.status}`);
    text = await response.text();
  }
  const rows = [];
  for (const [index, line] of text.split("\n").entries()) {
    if (line.trim().length === 0) continue;
    const where = `${name}:${index + 1}`;
    let row;
    try {
      row = JSON.parse(line);
    } catch (error) {
      throw new Error(`${where}: malformed dataset row`, { cause: error });
    }
    if (!isDatasetRow(row)) throw new Error(`${where}: malformed dataset row`);
    rows.push(row);
  }
  return rows;
}

// The fields `idOf` and `taskFromRow` read; `resolved_issues` may be absent
// but, when present, is an array.
function isDatasetRow(row) {
  return (
    isObject(row) &&
    typeof row.org === "string" &&
    typeof row.repo === "string" &&
    isObject(row.base) &&
    typeof row.base.sha === "string" &&
    typeof row.test_patch === "string" &&
    (typeof row.instance_id === "string" || typeof row.number === "number") &&
    (row.resolved_issues === undefined || Array.isArray(row.resolved_issues))
  );
}

function isObject(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function repoOf(instanceId) {
  const [owner, rest] = instanceId.split("__");
  return `${owner}/${rest.replace(/-\d+$/, "")}`;
}

/**
 * Build the tasks for `selection` in that order. Throws `no repo config for
 * <repo>` before reading any rows when an id's repo is not in REPOS,
 * `<file>:<line>: malformed dataset row` for a row that fails the field
 * checks, and when an id is missing from its repo's rows.
 */
async function buildTasks({
  fixtureDir,
  selection = SETS[DEFAULT_SET].ids,
} = {}) {
  const byRepo = new Map();
  for (const id of selection) {
    const repo = repoOf(id);
    if (!REPOS[repo]) throw new Error(`no repo config for ${repo}`);
    if (!byRepo.has(repo)) byRepo.set(repo, []);
    byRepo.get(repo).push(id);
  }
  const found = new Map();
  for (const [repo, ids] of byRepo) {
    const rows = await loadRows(repo, fixtureDir);
    for (const row of rows) {
      const id = idOf(row);
      if (ids.includes(id)) found.set(id, taskFromRow(row));
    }
  }
  return selection.map((id) => {
    const task = found.get(id);
    if (!task) throw new Error(`instance ${id} not in dataset`);
    return task;
  });
}

const USAGE = `usage: node bench/tasks/fetch-multi-swe-ts.mjs [--set ${Object.keys(SETS).join("|")}] [--fixture <dir>] [--out <file>] [--only id1,id2]`;

function readArgs(argv) {
  let values;
  try {
    ({ values } = parseArgs({
      args: argv,
      options: {
        set: { type: "string", default: DEFAULT_SET },
        fixture: { type: "string" },
        out: { type: "string" },
        only: { type: "string" },
      },
    }));
  } catch (error) {
    throw new Error(USAGE, { cause: error });
  }
  const set = SETS[values.set];
  if (!set) throw new Error(USAGE);
  return {
    fixtureDir: values.fixture,
    out:
      values.out ??
      path.join(path.dirname(fileURLToPath(import.meta.url)), set.out),
    selection: values.only?.split(",") ?? set.ids,
  };
}

/** CLI entry: writes the selected tasks as pretty JSON and returns how many. Throws the usage line on bad arguments. */
export async function main(argv = process.argv.slice(2)) {
  const args = readArgs(argv);
  const tasks = await buildTasks({
    fixtureDir: args.fixtureDir,
    selection: args.selection,
  });
  writeFileSync(args.out, `${JSON.stringify(tasks, null, 2)}\n`);
  return tasks.length;
}

const entry = process.argv[1];
if (entry && import.meta.url === pathToFileURL(path.resolve(entry)).href) {
  main().catch((error) => {
    process.stderr.write(`${errorLines(error)}\n`);
    process.exit(1);
  });
}

// Errors with a cause (the parseArgs failure under the usage text, a
// JSON.parse failure under a malformed row) print both lines.
function errorLines(error) {
  if (!(error instanceof Error)) return String(error);
  return error.cause instanceof Error
    ? `${error.message}\n${error.cause.message}`
    : error.message;
}
