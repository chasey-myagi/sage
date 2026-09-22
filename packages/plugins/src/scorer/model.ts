/**
 * Model scorer. One auxiliary call, only as a prefix extension.
 *
 * Contract: when `prevRequest` is missing, score resolves
 * `{ decisions: [], fallback: "no-previous-request" }` and does not call an
 * adapter. Otherwise the request copies that previous head and messages
 * byte for byte, keeps its tools, and appends one user message: a fixed
 * instruction, the query, and a JSON list of `{id, tool, summary, tokens}`.
 * Candidates beyond `maxCandidates` (default 60) drop the oldest. The
 * instruction requires JSON only, tier `none` | `summary` | `full`, and a
 * summary of at most 200 characters. `timeoutMs` defaults to 20000. Invalid
 * JSON, timeout, a missing adapter, and a finish error resolve `fallback`
 * and no decisions. A completed call returns the parsed decisions and the
 * stream usage.
 *
 * Intentionally not in the Log: this call. The compiler copies usage and
 * fallback onto the manifest; the kernel stores that manifest.
 */

import {
  collect,
  type ModelRef,
  type ProviderAdapter,
  type Request,
} from "@sage/provider/contract";
import { summarizeInput } from "../compiler/stub.js";
import type { ScoreDecision, Scorer } from "./types.js";

const SUMMARY_LIMIT = 200;
const DEFAULT_TIMEOUT_MS = 20_000;
const DEFAULT_MAX_CANDIDATES = 60;

const INSTRUCTION = [
  "Output JSON only. No other text.",
  "tier must be one of none, summary, full.",
  "summary must be at most 200 characters.",
].join("\n");

const TIERS = new Set(["none", "summary", "full"]);

export function modelScorer(opts: {
  adapters: Record<string, ProviderAdapter>;
  model: ModelRef;
  timeoutMs?: number;
  maxCandidates?: number;
}): Scorer {
  const timeoutMs = opts.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  const maxCandidates = opts.maxCandidates ?? DEFAULT_MAX_CANDIDATES;

  return {
    name: "model",
    async score(input) {
      if (!input.prevRequest) {
        return { decisions: [], fallback: "no-previous-request" };
      }
      const adapter = opts.adapters[opts.model.provider];
      if (!adapter) return { decisions: [], fallback: "no-adapter" };

      const limit = maxCandidates <= 0 ? 0 : maxCandidates;
      const selected =
        limit === 0
          ? []
          : input.candidates.slice(
              Math.max(0, input.candidates.length - limit),
            );
      const listed = selected.map((chunk) => ({
        id: chunk.id,
        tool: chunk.toolName ?? "",
        summary: summarizeInput(chunk.input),
        tokens: chunk.tokens,
      }));
      const text = [
        INSTRUCTION,
        `Query: ${input.query}`,
        JSON.stringify(listed),
      ].join("\n");
      const messages = structuredClone(input.prevRequest.messages);
      messages.push({ role: "user", content: [{ type: "text", text }] });
      const request: Request = {
        model: opts.model,
        head: structuredClone(input.prevRequest.head),
        messages,
        cacheBreakpoints: structuredClone(input.prevRequest.cacheBreakpoints),
        sampling: { ...input.prevRequest.sampling },
      };

      const controller = new AbortController();
      let cause: "timeout" | "aborted" | undefined;
      const timer = setTimeout(() => {
        cause = cause ?? "timeout";
        controller.abort();
      }, timeoutMs);
      const onCallerAbort = () => {
        cause = cause ?? "aborted";
        controller.abort();
      };
      if (input.signal?.aborted) onCallerAbort();
      else
        input.signal?.addEventListener("abort", onCallerAbort, { once: true });

      const collected = collect(
        adapter.stream(request, adapter.prepare(opts.model), controller.signal),
        {
          responseModel: opts.model.model,
          responseId: "",
          api: adapter.api,
        },
      ).then(
        (value) => ({ ok: true as const, value }),
        (error: unknown) => ({ ok: false as const, error }),
      );
      const aborted = new Promise<{ ok: false; error: unknown }>((resolve) => {
        const finish = () =>
          resolve({ ok: false, error: new Error(cause ?? "aborted") });
        if (controller.signal.aborted) finish();
        else {
          controller.signal.addEventListener("abort", finish, { once: true });
        }
      });

      try {
        const raced = await Promise.race([collected, aborted]);
        if (!raced.ok) {
          return {
            decisions: [],
            fallback:
              cause === "timeout"
                ? "timeout"
                : cause === "aborted"
                  ? "aborted"
                  : "error",
          };
        }
        const assistant = raced.value;
        if (cause === "timeout") {
          return {
            decisions: [],
            usage: assistant.usage,
            fallback: "timeout",
          };
        }
        if (cause === "aborted" || assistant.finish.kind === "aborted") {
          return {
            decisions: [],
            usage: assistant.usage,
            fallback: "aborted",
          };
        }
        if (assistant.finish.kind === "error") {
          return {
            decisions: [],
            usage: assistant.usage,
            fallback: "finish:error",
          };
        }
        const body = assistant.content
          .filter((block) => block.type === "text")
          .map((block) => block.text)
          .join("");
        const decisions = parseDecisions(body);
        if (!decisions) {
          return {
            decisions: [],
            usage: assistant.usage,
            fallback: "invalid-json",
          };
        }
        return { decisions, usage: assistant.usage };
      } finally {
        clearTimeout(timer);
        input.signal?.removeEventListener("abort", onCallerAbort);
      }
    },
  };
}

function parseDecisions(text: string): ScoreDecision[] | undefined {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text.trim());
  } catch {
    return undefined;
  }
  if (!isRecord(parsed) || !Array.isArray(parsed.decisions)) return undefined;
  const decisions: ScoreDecision[] = [];
  for (const item of parsed.decisions) {
    if (
      !isRecord(item) ||
      typeof item.id !== "string" ||
      item.id.length === 0
    ) {
      return undefined;
    }
    if (typeof item.tier !== "string" || !TIERS.has(item.tier))
      return undefined;
    const decision: ScoreDecision = {
      id: item.id,
      tier: item.tier as ScoreDecision["tier"],
    };
    if (item.summary !== undefined) {
      if (typeof item.summary !== "string") return undefined;
      decision.summary = clampSummary(item.summary);
    }
    decisions.push(decision);
  }
  return decisions;
}

function clampSummary(summary: string): string {
  if (summary.length <= SUMMARY_LIMIT) return summary;
  return summary.slice(0, SUMMARY_LIMIT);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
