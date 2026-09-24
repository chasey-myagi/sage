/**
 * Escape untrusted text before it is wrapped in a claim tag.
 *
 * Contract: the character sequence `</claim` is replaced with `<\/claim`.
 * The compiler's claim parser closes on that sequence, so an earlier
 * occurrence would end the claim and leave the rest of the text outside.
 *
 * Intentionally not in the Log: this function. The escaped string is what
 * a claim provider puts on a tier; the compiler logs the rendered claim.
 */

/** Returns `text` with every `</claim` replaced by `<\/claim`. */
export function escapeClaimText(text: string): string {
  return text.replaceAll("</claim", "<\\/claim");
}
