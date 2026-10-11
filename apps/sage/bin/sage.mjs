#!/usr/bin/env node
/**
 * Starts the one-shot CLI from the bundle `dist/cli.js`.
 *
 * Contract: exits with the code `runCli` returns. The bundle is built by
 * `pnpm build` (and by `prepack`); this file does not build or fall back to
 * TypeScript source.
 *
 * Intentionally not in the Log: process startup.
 */

import { runCli } from "../dist/cli.js";

process.exit(await runCli(process.argv.slice(2)));
