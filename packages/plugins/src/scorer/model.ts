/**
 * Model scorer. One auxiliary call, only as a prefix extension.
 *
 * Contract: when `prevRequest` is missing, score resolves
 * `{ decisions: [], fallback: "no-previous-request" }` and does not call an
 * adapter. Otherwise the request copies that previous head and messages
 * byte for byte, keeps its tools, sets `toolChoice` to `"none"`, and
 * appends one user message: a fixed instruction, the query, and a JSON
 * list of `{id, tool, summary, tokens}`. Candidates beyond `maxCandidates`
 * (default 60) drop the oldest. The instruction says this is a side
 * judgment of earlier tool results, not a continuation of the task, and
 * that the reply is a JSON array of `{id, tier, summary?}`. `timeoutMs`
 * defaults to 20000. The parser strips one fenced json block, wraps a single
 * object that has `id` into a one-item array, drops items whose id is not
 * in the listed candidates (`dropped`), and treats `[]` as keep-all.
 * A malformed item fails the parse. Invalid JSON, timeout, a missing
 * adapter, and a finish error resolve `fallback` and no decisions. When
 * that fallback follows a model reply, `raw` is `{ text, finish }` with at
 * most 300 characters of text. A completed call returns the parsed
 * decisions and the stream usage, and does not set `raw`.
 *
 * Intentionally not in the Log: this call, and `dropped`. The compiler
 * copies usage, fallback, and `raw` onto the manifest; the kernel stores
 * that manifest.
 */

import {
  collect,
  type ModelRef,
  type ProviderAdapter,
  type Request,
} from "@sage/provider/contract";
import { summarizeInput } from "../compiler/stub.js";
import type { ScoreDecision, ScoreRaw, Scorer } from "./types.js";

const SUMMARY_LIMIT = 200;
const RAW_LIMIT = 300;
const DEFAULT_TIMEOUT_MS = 20_000;
const DEFAULT_MAX_CANDIDATES = 60;

const INSTRUCTION = [
  "You are judging which earlier tool results can be dropped from the context.",
  "This is a side query, not a continuation of the task.",
  "do not call tools. Do not output any other text.",
  "Each candidate is the result of an earlier tool call.",
  "Output a JSON array only.",
  'Each item is {"id":"<candidate id>","tier":"none"|"summary"|"full","summary"?:string}.',
  "Copy id from the candidate list.",
  "summary is optional and at most 200 characters.",
  "tier none drops the result, summary keeps a short note, full keeps the original text.",
  "Example:",
  '[{"id":"seq:1#a","tier":"none"},{"id":"seq:2#b","tier":"summary","summary":"listed the files"}]',
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
        toolChoice: "none",
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
        const body = assistant.content
          .filter((block) => block.type === "text")
          .map((block) => block.text)
          .join("");
        const raw = replyRaw(body, assistant.finish.kind);
        if (cause === "timeout") {
          return {
            decisions: [],
            usage: assistant.usage,
            fallback: "timeout",
            raw,
          };
        }
        if (cause === "aborted" || assistant.finish.kind === "aborted") {
          return {
            decisions: [],
            usage: assistant.usage,
            fallback: "aborted",
            raw,
          };
        }
        if (assistant.finish.kind === "error") {
          return {
            decisions: [],
            usage: assistant.usage,
            fallback: "finish:error",
            raw,
          };
        }
        const parsed = parseDecisions(
          body,
          new Set(selected.map((chunk) => chunk.id)),
        );
        if (!parsed) {
          return {
            decisions: [],
            usage: assistant.usage,
            fallback: "invalid-json",
            raw,
          };
        }
        return {
          decisions: parsed.decisions,
          usage: assistant.usage,
          ...(parsed.dropped > 0 ? { dropped: parsed.dropped } : {}),
        };
      } finally {
        clearTimeout(timer);
        input.signal?.removeEventListener("abort", onCallerAbort);
      }
    },
  };
}

function replyRaw(text: string, finish: string): ScoreRaw {
  return { text: text.slice(0, RAW_LIMIT), finish };
}

function parseDecisions(
  text: string,
  allowed: ReadonlySet<string>,
): { decisions: ScoreDecision[]; dropped: number } | undefined {
  let parsed: unknown;
  try {
    parsed = JSON.parse(stripFence(text));
  } catch {
    return undefined;
  }
  const items = asItems(parsed);
  if (!items) return undefined;
  const decisions: ScoreDecision[] = [];
  let dropped = 0;
  for (const item of items) {
    const read = readItem(item, allowed);
    if (read.kind === "fail") return undefined;
    if (read.kind === "drop") {
      dropped += 1;
      continue;
    }
    decisions.push(read.decision);
  }
  return { decisions, dropped };
}

function stripFence(text: string): string {
  const trimmed = text.trim();
  const fenced = /^```(?:json)?\s*([\s\S]*?)```$/i.exec(trimmed);
  if (!fenced || fenced[1] === undefined) return trimmed;
  return fenced[1].trim();
}

function asItems(parsed: unknown): unknown[] | undefined {
  if (Array.isArray(parsed)) return parsed;
  if (
    isRecord(parsed) &&
    typeof parsed.id === "string" &&
    parsed.id.length > 0
  ) {
    return [parsed];
  }
  return undefined;
}

function readItem(
  item: unknown,
  allowed: ReadonlySet<string>,
):
  | { kind: "keep"; decision: ScoreDecision }
  | { kind: "drop" }
  | { kind: "fail" } {
  if (!isRecord(item)) return { kind: "fail" };
  if (typeof item.id !== "string" || item.id.length === 0) {
    return { kind: "fail" };
  }
  if (!allowed.has(item.id)) return { kind: "drop" };
  if (typeof item.tier !== "string" || !TIERS.has(item.tier)) {
    return { kind: "fail" };
  }
  if (item.summary !== undefined && typeof item.summary !== "string") {
    return { kind: "fail" };
  }
  const decision: ScoreDecision = {
    id: item.id,
    tier: item.tier as ScoreDecision["tier"],
  };
  if (typeof item.summary === "string") {
    decision.summary = clampSummary(item.summary);
  }
  return { kind: "keep", decision };
}

function clampSummary(summary: string): string {
  if (summary.length <= SUMMARY_LIMIT) return summary;
  return summary.slice(0, SUMMARY_LIMIT);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
