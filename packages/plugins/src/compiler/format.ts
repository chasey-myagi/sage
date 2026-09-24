/**
 * Fixed claim rendering. Epoch-head segments and tail blocks share this text.
 *
 * Contract: one claim is `<claim id="…" tier="…">`, the tier text, then
 * `</claim>`. Several claims in one message are joined with a blank line.
 * The content hash is sha256 of that exact string.
 *
 * Intentionally not in the Log: these helpers. The rendered string is what
 * gets stored, either as `view_epoch.details.prefixMessage` or as
 * `manifest.details.claimContent`.
 */

import { createHash } from "node:crypto";
import type { Message } from "@sage/provider/contract";
import { isTier } from "../guards.js";
import type { Tier } from "./claim.js";

/** Renders one claim as `<claim id="…" tier="…">`, the body, `</claim>`. */
export function renderClaim(id: string, tier: string, body: string): string {
  return `<claim id="${id}" tier="${tier}">\n${body}\n</claim>`;
}

/** Hex sha256 of `text`. */
export function contentHash(text: string): string {
  return createHash("sha256").update(text).digest("hex");
}

/**
 * Extracts every rendered claim from a message's text blocks, keyed by id.
 * A later claim with the same id replaces an earlier one.
 */
export function parseClaimMessage(
  message: Message,
): Map<string, { tier: Tier; body: string }> {
  const found = new Map<string, { tier: Tier; body: string }>();
  const pattern =
    /<claim id="([^"]*)" tier="(none|line|summary|full)">\n([\s\S]*?)\n<\/claim>/g;
  for (const block of message.content) {
    if (block.type !== "text") continue;
    for (const match of block.text.matchAll(pattern)) {
      const id = match[1];
      const tier = match[2];
      const body = match[3];
      if (!id || !tier || body === undefined || !isTier(tier)) continue;
      found.set(id, { tier, body });
    }
  }
  return found;
}
