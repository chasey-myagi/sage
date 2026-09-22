/**
 * CLI package entry.
 *
 * Intentionally not in the Log: package identity. The session log is written
 * by createCodingSession, not by this re-export.
 */

export const PACKAGE = "sage" as const;

export { type CodingSession, createCodingSession, runCli } from "./main.js";
