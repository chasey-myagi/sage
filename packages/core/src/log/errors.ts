/**
 * Failures while appending or reopening a log.
 *
 * Guarantees: a torn last line is not one of these errors. Mid-file damage and
 * unknown kinds refuse the reopen. A payload that fails its kind's validate
 * function never reaches disk.
 *
 * Intentionally not in the Log: these errors are runtime signals, not entries.
 */

/** A JSONL line other than the last is unreadable. `line` is 1-based. */
export class LogCorruptedError extends Error {
  readonly line: number;

  constructor(line: number) {
    super(`Log corrupted at line ${line}`);
    this.name = "LogCorruptedError";
    this.line = line;
  }
}

/** An entry names a kind no module registered. */
export class UnknownEntryKindError extends Error {
  readonly kind: string;

  constructor(kind: string) {
    super(`Unknown entry kind: ${kind}`);
    this.name = "UnknownEntryKindError";
    this.kind = kind;
  }
}

/** A payload failed its kind's validator, or a `leaf` target is unknown. */
export class InvalidEntryError extends Error {
  readonly kind: string;

  constructor(kind: string) {
    super(`Invalid entry payload for kind ${kind}`);
    this.name = "InvalidEntryError";
    this.kind = kind;
  }
}
