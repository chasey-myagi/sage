/**
 * Process handles every subcommand writes through, so tests can capture
 * stdout and stderr without touching the real process.
 *
 * Intentionally not in the Log: anything written here. stdout carries the
 * command's JSON result; stderr carries progress and errors.
 */

import type { ProviderAdapter } from "@sage/provider";
import type { BashExec } from "@sage/tools";
import type { BenchDeps } from "./bench/types.js";
import type { JevFetch } from "./scorers.js";

/** A text sink such as `process.stdout`. */
export type TextOut = { write(chunk: string): void };

/** Working directory and output streams for one subcommand. */
export type CommandIo = {
  cwd: string;
  stdout: TextOut;
  stderr: TextOut;
};

/**
 * Everything `runCli` reads from the process, each field overridable.
 * Omitted fields fall back to the real process.
 */
export type CliIo = CommandIo & {
  env: NodeJS.ProcessEnv;
  /** Adapter for the selected provider. Test seam; production builds from env. */
  adapter?: ProviderAdapter;
  /** Adapters by provider. Test seam; production builds one per keyed preset. */
  adapters?: Record<string, ProviderAdapter>;
  /** Test seam. Production runs use git, the sage bin, and the test command. */
  benchDeps?: BenchDeps;
  /** Test seam. Production bash spawns real processes. */
  exec?: BashExec;
  /** Test seam. Production uses process.platform. */
  platform?: NodeJS.Platform;
  /** Test seam. Production probes `unshare -n` on linux. */
  unshareAvailable?: boolean;
  /** Test seam. Production jev scoring uses global fetch. */
  jevFetch?: JevFetch;
};
