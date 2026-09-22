#!/usr/bin/env node
/**
 * Starts the one-shot CLI.
 *
 * Workspace packages are TypeScript source. Node must transform types, and
 * relative imports use .js specifiers, so this process re-execs with
 * --experimental-transform-types and a resolver that falls back to .ts.
 *
 * Intentionally not in the Log: process startup.
 */

import { spawnSync } from "node:child_process";
import { register } from "node:module";

const FLAG = "--experimental-transform-types";

if (!process.execArgv.includes(FLAG)) {
  const entry = process.argv[1] ?? "";
  const result = spawnSync(
    process.execPath,
    [FLAG, ...process.execArgv, entry, ...process.argv.slice(2)],
    { stdio: "inherit", env: process.env },
  );
  process.exit(result.status ?? 1);
}

register(new URL("../src/resolve-ts.mjs", import.meta.url));

const { runCli } = await import("../src/main.ts");
const code = await runCli(process.argv.slice(2));
process.exit(code);
