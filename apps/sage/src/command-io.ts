/**
 * Output handles shared by every subcommand, the bench included. Kept apart
 * from `io.ts` so bench types can use them without importing the full CLI
 * handle, which itself refers to bench types.
 *
 * Intentionally not in the Log: anything written here. stdout carries the
 * command's JSON result; stderr carries progress and errors.
 */

/** A text sink such as `process.stdout`. */
export type TextOut = { write(chunk: string): void };

/** Working directory and output streams for one subcommand. */
export type CommandIo = {
  cwd: string;
  stdout: TextOut;
  stderr: TextOut;
};
