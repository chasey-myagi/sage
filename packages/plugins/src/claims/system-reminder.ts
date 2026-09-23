/**
 * System reminder claim.
 *
 * Contract: one tail claim, line tier only, default priority 20. The text
 * is escaped so it cannot close the claim tag. There is no other tier.
 *
 * Intentionally not in the Log: the provider closure. The line reaches the
 * Log only as manifest claim content written by the compiler.
 */

import type { ClaimProvider } from "../compiler/claim.js";
import { escapeClaimText } from "./escape.js";

/** Default priority of the system reminder claim. */
const DEFAULT_REMINDER_PRIORITY = 20;

/** Creates the claim provider for claim `system-reminder`. */
export function systemReminder(opts: {
  text: string;
  priority?: number;
}): ClaimProvider {
  return () => [
    {
      id: "system-reminder",
      priority: opts.priority ?? DEFAULT_REMINDER_PRIORITY,
      placement: "tail",
      tiers: { line: escapeClaimText(opts.text) },
    },
  ];
}
