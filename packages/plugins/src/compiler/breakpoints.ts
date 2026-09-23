/**
 * Cache breakpoints for a compiled view.
 *
 * Contract: a view has three breakpoints: `head`, the end of the frozen
 * prefix, and the last message. Inside an epoch the frozen prefix ends at
 * the last history message whose Log seq is at or below the epoch's seq
 * ceiling; on a rebuild or an epoch's first turn it ends at the last
 * history message. Indexes are clamped into the message list.
 *
 * Intentionally not in the Log: the breakpoint indexes. The rebuild path
 * recomputes breakpoints from the rebuilt messages.
 */

import type { CompiledView } from "@sage/core";

/**
 * Returns the three breakpoints. `seqs` maps history messages to Log seqs
 * (-1 for a synthetic repair message, which follows the real message
 * before it). `ceiling` is undefined when the whole history is the prefix.
 */
export function cacheBreakpointsFor(input: {
  messageCount: number;
  historyCount: number;
  hasEpochHead: boolean;
  seqs: readonly number[];
  ceiling: number | undefined;
}): CompiledView["cacheBreakpoints"] {
  const end = prefixEndIndex(input);
  const last = Math.max(0, input.messageCount - 1);
  return ["head", { afterMessage: end }, { afterMessage: last }];
}

function prefixEndIndex(input: {
  messageCount: number;
  historyCount: number;
  hasEpochHead: boolean;
  seqs: readonly number[];
  ceiling: number | undefined;
}): number {
  const { messageCount, historyCount, seqs, ceiling } = input;
  if (messageCount === 0) return 0;
  const offset = input.hasEpochHead ? 1 : 0;
  if (historyCount === 0) return Math.min(messageCount - 1, offset);
  if (ceiling === undefined) {
    return Math.min(messageCount - 1, offset + historyCount - 1);
  }
  let lastHistory = -1;
  let prevReal: number | undefined;
  for (let index = 0; index < historyCount; index += 1) {
    const seq = seqs[index] ?? 0;
    const included =
      seq >= 0 ? seq <= ceiling : prevReal !== undefined && prevReal <= ceiling;
    if (seq >= 0) prevReal = seq;
    if (included) lastHistory = index;
  }
  if (lastHistory < 0) return Math.min(messageCount - 1, offset);
  return Math.min(messageCount - 1, offset + lastHistory);
}
