/**
 * CLI package entry.
 *
 * Intentionally not in the Log: package identity. The session log is written
 * by createCodingSession, not by this re-export.
 */

/** Package name. */
export const PACKAGE = "sage" as const;

export {
  type CodingSession,
  createCodingSession,
} from "./coding-session.js";
export { runCli } from "./main.js";
