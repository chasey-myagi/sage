/**
 * Progress ledger claim and the tool that records it.
 *
 * Contract: `update_ledger` does not validate input and does not write
 * anywhere. execute returns `Ledger updated.` The provider folds every
 * assistant tool_use named `update_ledger` on the path. A later call
 * replaces only the fields it includes; plan and decisions replace the
 * whole array. The claim is pinned, tail-if-changed, priority 100, full
 * tier only. No such call, no claim. Missing sections are omitted.
 *
 * Intentionally not in the Log: the tool object and the folded frame.
 * The acknowledgement is the tool_result the session already logs. The
 * ledger text reaches the Log only as manifest claim content.
 */

import { type HarnessTool, isRecord } from "@sage/core";
import type { ClaimProvider } from "../compiler/claim.js";
import { escapeClaimText } from "./escape.js";

type Frame = {
  goal?: string;
  plan?: string[];
  decisions?: string[];
};

const inputSchema = {
  type: "object",
  properties: {
    goal: {
      type: "string",
      description: "Task goal. Replaces the previous goal when included.",
    },
    plan: {
      type: "array",
      items: { type: "string" },
      description: "Plan steps. Replaces the previous plan when included.",
    },
    decisions: {
      type: "array",
      items: { type: "string" },
      description:
        "Decisions already made. Replaces the previous list when included.",
    },
  },
};

/**
 * Model-visible name of the ledger tool; the claim reads its calls back
 * from the Log by this name, and the rule scorer tiers its results by it.
 */
export const LEDGER_TOOL_NAME = "update_ledger";
/** Default priority of the ledger claim. */
const DEFAULT_LEDGER_PRIORITY = 100;

/**
 * Creates the `update_ledger` tool and the claim provider that folds its
 * calls from the path. The provider rereads the whole path on every
 * compile.
 */
export function progressLedger(opts?: { priority?: number }): {
  tool: HarnessTool;
  provider: ClaimProvider;
} {
  const priority = opts?.priority ?? DEFAULT_LEDGER_PRIORITY;
  const tool: HarnessTool = {
    def: {
      name: LEDGER_TOOL_NAME,
      description:
        "Record the task frame. Fields included on a call replace the previous goal, plan, or decisions.",
      inputSchema,
    },
    isReadOnly: true,
    isDestructive: false,
    isOpenWorld: false,
    accesses() {
      return [];
    },
    execute() {
      return Promise.resolve({ content: "Ledger updated.", isError: false });
    },
  };

  const provider: ClaimProvider = (ctx) => {
    const frame: Frame = {};
    let seen = false;
    for (const entry of ctx.path) {
      if (entry.kind !== "message") continue;
      const payload = entry.payload;
      if (!isRecord(payload) || payload.role !== "assistant") continue;
      if (!Array.isArray(payload.content)) continue;
      for (const block of payload.content) {
        if (!isRecord(block) || block.type !== "tool_use") continue;
        if (block.name !== LEDGER_TOOL_NAME) continue;
        seen = true;
        apply(frame, block.input);
      }
    }
    const text = renderFrame(frame);
    if (!seen || text === "") return [];
    return [
      {
        id: "progress-ledger",
        priority,
        pinned: true,
        placement: "tail-if-changed",
        tiers: { full: escapeClaimText(text) },
      },
    ];
  };

  return { tool, provider };
}

function apply(frame: Frame, input: unknown): void {
  if (!isRecord(input)) return;
  if (typeof input.goal === "string") frame.goal = input.goal;
  if (isStringArray(input.plan)) frame.plan = input.plan.slice();
  if (isStringArray(input.decisions)) frame.decisions = input.decisions.slice();
}

function renderFrame(frame: Frame): string {
  const parts: string[] = [];
  if (frame.goal !== undefined) parts.push(`Goal: ${frame.goal}`);
  if (frame.plan !== undefined) {
    parts.push(["Plan:", ...frame.plan.map((item) => `- ${item}`)].join("\n"));
  }
  if (frame.decisions !== undefined) {
    parts.push(
      ["Decisions:", ...frame.decisions.map((item) => `- ${item}`)].join("\n"),
    );
  }
  return parts.join("\n");
}

function isStringArray(value: unknown): value is string[] {
  return (
    Array.isArray(value) && value.every((item) => typeof item === "string")
  );
}
