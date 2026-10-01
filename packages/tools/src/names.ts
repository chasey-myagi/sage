/**
 * Names of the first-party tools, as the model sees them in the schema.
 *
 * Contract: each tool's `def.name` is the matching value here, so callers
 * that read tool calls back from the Log compare against these instead of
 * spelling the names again.
 *
 * Intentionally not in the Log: this lookup; tool calls already log names.
 */

/** Tool name by tool. */
export const TOOL_NAMES = {
  read: "read",
  write: "write",
  edit: "edit",
  bash: "bash",
  grep: "grep",
  find: "find",
  ls: "ls",
} as const;
