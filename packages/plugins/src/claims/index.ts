/**
 * First-party claim providers.
 *
 * Contract: callers receive claims or, for the ledger, a tool plus a
 * provider. Nothing here appends to the Log or edits the session head.
 *
 * Intentionally not in the Log: this barrel. Claim text reaches the Log
 * only through the manifest entry the compiler asks the kernel to append.
 */

export { agentsMd } from "./agents-md.js";
export { escapeClaimText } from "./escape.js";
export { progressLedger } from "./progress-ledger.js";
export { systemReminder } from "./system-reminder.js";
export { toolCatalog } from "./tool-catalog.js";
