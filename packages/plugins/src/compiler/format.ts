/**
 * Fixed claim rendering. Epoch-head segments and tail blocks share this text.
 *
 * Contract: one claim is `<claim id="…" tier="…">`, the tier text, then
 * `</claim>`. Several claims in one message are joined with a blank line.
 * The content hash is sha256 of that exact string.
 *
 * Intentionally not in the Log: these helpers. The rendered string is what
 * gets stored, either as `view_epoch.details.epochHead` or as
 * `manifest.details.claimContent`.
 */

import { createHash } from "node:crypto";
import type { Message } from "@sage/provider/contract";
import type { Tier } from "./claim.js";

const TIERS = new Set<Tier>(["none", "line", "summary", "full"]);

export function renderClaim(id: string, tier: string, body: string): string {
  return `<claim id="${id}" tier="${tier}">\n${body}\n</claim>`;
}

export function contentHash(text: string): string {
  return createHash("sha256").update(text).digest("hex");
}

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

function isTier(value: string): value is Tier {
  return TIERS.has(value as Tier);
}
