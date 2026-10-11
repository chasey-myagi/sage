// Packs the published CLI, installs the tarball into an empty directory, and
// runs `sage --help` from there with no extra Node flags.
// Not model-visible and not a Log entry: this only checks the package shape.
// Two checks keep the tarball self-contained: its package.json must not have
// a `dependencies` field, and the install runs `npm install --offline`
// against an empty cache, so any runtime dependency fails to install.
import { spawnSync } from "node:child_process";
import {
  mkdirSync,
  mkdtempSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const PACKAGE_NAME = "@chasey-myagi/sage";
const USAGE_PREFIX = "usage: sage";

const repoRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);

/**
 * Runs `command` and returns its result; throws with the captured output
 * when it exits non-zero.
 * @param {string} command
 * @param {string[]} args
 * @param {string} cwd
 */
function run(command, args, cwd) {
  const result = spawnSync(command, args, { cwd, encoding: "utf8" });
  if (result.status !== 0) {
    throw new Error(
      `${command} ${args.join(" ")} exited ${result.status}\n${result.stdout}${result.stderr}`,
    );
  }
  return result;
}

const work = mkdtempSync(path.join(tmpdir(), "sage-smoke-"));
try {
  const packDir = path.join(work, "pack");
  run(
    "pnpm",
    ["--filter", PACKAGE_NAME, "pack", "--pack-destination", packDir],
    repoRoot,
  );
  const tarballs = readdirSync(packDir).filter((name) => name.endsWith(".tgz"));
  if (tarballs.length !== 1) {
    throw new Error(`expected one tarball, found ${tarballs.length}`);
  }
  const tarball = path.join(packDir, tarballs[0]);
  const manifest = run(
    "tar",
    ["-xOzf", tarball, "package/package.json"],
    work,
  ).stdout;
  if ("dependencies" in JSON.parse(manifest)) {
    throw new Error(
      `${tarballs[0]} declares dependencies; the bundle must carry them`,
    );
  }

  // An empty cache: the user's npm cache would satisfy --offline otherwise.
  const cacheDir = path.join(work, "npm-cache");
  mkdirSync(cacheDir);

  const installDir = path.join(work, "install");
  mkdirSync(installDir);
  writeFileSync(path.join(installDir, "package.json"), '{ "private": true }\n');
  run(
    "npm",
    [
      "install",
      "--offline",
      "--cache",
      cacheDir,
      "--no-audit",
      "--no-fund",
      "--ignore-scripts",
      tarball,
    ],
    installDir,
  );

  // `--help` prints usage to stderr and exits 1 by design (see the app
  // README), so the check reads stderr and ignores the exit code.
  const env = { ...process.env };
  delete env.NODE_OPTIONS;
  const help = spawnSync(
    path.join(installDir, "node_modules", ".bin", "sage"),
    ["--help"],
    { cwd: installDir, env, encoding: "utf8" },
  );
  if (!help.stderr.startsWith(USAGE_PREFIX)) {
    throw new Error(
      `sage --help did not print usage\nstdout: ${help.stdout}\nstderr: ${help.stderr}`,
    );
  }
  process.stdout.write(
    `smoke ok: ${tarballs[0]} -> ${help.stderr.split("\n")[0].slice(0, 60)}...\n`,
  );
} finally {
  rmSync(work, { recursive: true, force: true });
}
