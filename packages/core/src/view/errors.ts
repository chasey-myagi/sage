/**
 * Seam failures. Thrown before a request is sent.
 *
 * Guarantees: an unpaired view or a mid-epoch head change is a compiler bug.
 * The session records a terminal error entry, then throws. These errors are
 * not log entries by themselves.
 *
 * Intentionally not in the Log: the Error instance. The terminal entry stores
 * the outcome and the message.
 */

/** A tool_use without its result, or a result without its call. `id` names it. */
export class UnpairedToolCallError extends Error {
  readonly id: string;

  constructor(id: string) {
    super(`Unpaired tool call: ${id}`);
    this.name = "UnpairedToolCallError";
    this.id = id;
  }
}

/** The view changed system or tools without opening a new epoch first. */
export class HeadChangedMidEpochError extends Error {
  constructor() {
    super(
      "Head changed mid-epoch; system and tools may change only at a view_epoch boundary",
    );
    this.name = "HeadChangedMidEpochError";
  }
}
