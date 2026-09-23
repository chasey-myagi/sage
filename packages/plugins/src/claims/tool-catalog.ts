/**
 * Deferred-tool catalog claim.
 *
 * Contract: an empty list submits nothing. Otherwise one epoch-head claim,
 * default priority 40. The line tier is one `- name — description` row per
 * tool. The full tier is that text plus each tool's inputSchema JSON.
 * This provider does not edit the session head, so it does not activate
 * tools.
 *
 * Intentionally not in the Log: the deferred list. Catalog text reaches the
 * Log only as manifest claim content written by the compiler.
 */

import type { ToolDef } from "@sage/provider/contract";
import type { ClaimProvider } from "../compiler/claim.js";
import { escapeClaimText } from "./escape.js";

/** Default priority of the tool catalog claim. */
const DEFAULT_CATALOG_PRIORITY = 40;

/** Creates the claim provider for claim `tool-catalog` over `deferred`. */
export function toolCatalog(opts: {
  deferred: readonly ToolDef[];
  priority?: number;
}): ClaimProvider {
  return () => {
    if (opts.deferred.length === 0) return [];
    const line = opts.deferred
      .map((tool) => `- ${tool.name} — ${tool.description}`)
      .join("\n");
    const schemas = opts.deferred
      .map(
        (tool) => `${tool.name}\n${JSON.stringify(tool.inputSchema, null, 2)}`,
      )
      .join("\n\n");
    const full = `${line}\n\n${schemas}`;
    return [
      {
        id: "tool-catalog",
        priority: opts.priority ?? DEFAULT_CATALOG_PRIORITY,
        placement: "epoch-head",
        tiers: {
          line: escapeClaimText(line),
          full: escapeClaimText(full),
        },
      },
    ];
  };
}
