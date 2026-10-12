/**
 * What the jev scorer reads out of `ScoreInput.prevRequest.messages`: each
 * candidate's original tool_result text, the assistant prose that issued
 * the call, and the progress ledger claim.
 *
 * Guarantees: reading never mutates the messages. A result that is the
 * compiler's own stub or summary rendering counts as missing, because it
 * no longer holds the original text.
 *
 * Intentionally not in the Log: these readers. The messages they read are
 * already a request the Log rebuilds.
 */

import type { Message } from "@sage/provider/contract";
import type { Chunk } from "../../compiler/chunks.js";
import { parseClaimMessage } from "../../compiler/format.js";
import { renderStub } from "../../compiler/stub.js";

/**
 * Id of the claim whose body is the folded progress ledger. It is the id
 * the `progressLedger` claim provider writes; the scorer reads the rendered
 * claim text instead of importing that plugin.
 */
const LEDGER_CLAIM_ID = "progress-ledger";

/** Lookups over one request's messages. */
export type RequestSources = {
  /** The newest ledger claim body, or "" when the request has none. */
  ledger: string;
  /**
   * The chunk's tool_result content, or undefined when the request has no
   * result for its toolUseId or the result is the stub (with or without a
   * summary note).
   */
  resultText(chunk: Chunk): string | undefined;
  /**
   * The text blocks of the assistant message that holds the chunk's
   * tool_use, joined with newlines; "" when that message has no text.
   */
  assistantProse(chunk: Chunk): string;
};

/** Indexes `messages` once for the per-chunk lookups. */
export function readSources(messages: readonly Message[]): RequestSources {
  const results = new Map<string, string>();
  const prose = new Map<string, string>();
  let ledger = "";
  for (const message of messages) {
    const claim = parseClaimMessage(message).get(LEDGER_CLAIM_ID);
    if (claim) ledger = claim.body;
    indexMessage(message, results, prose);
  }
  return {
    ledger,
    resultText(chunk) {
      if (chunk.toolUseId === undefined) return undefined;
      const text = results.get(chunk.toolUseId);
      // The stub embeds the chunk id and size, so only this chunk's own
      // stub, rendered from the same Log facts, can match it.
      if (text === undefined || text.startsWith(renderStub(chunk))) {
        return undefined;
      }
      return text;
    },
    assistantProse(chunk) {
      if (chunk.toolUseId === undefined) return "";
      return prose.get(chunk.toolUseId) ?? "";
    },
  };
}

/** First result per toolUseId wins, matching the chunk index. */
function indexMessage(
  message: Message,
  results: Map<string, string>,
  prose: Map<string, string>,
): void {
  if (message.role === "assistant") {
    const text = message.content
      .flatMap((block) => (block.type === "text" ? [block.text] : []))
      .join("\n");
    for (const block of message.content) {
      if (block.type === "tool_use" && !prose.has(block.id)) {
        prose.set(block.id, text);
      }
    }
    return;
  }
  for (const block of message.content) {
    if (block.type === "tool_result" && !results.has(block.toolUseId)) {
      results.set(block.toolUseId, block.content);
    }
  }
}
