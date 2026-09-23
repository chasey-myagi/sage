/**
 * Model scorer: one auxiliary call that extends the previous main request
 * as a prefix, so it reads that request's prompt cache.
 *
 * Guarantees: the auxiliary request repeats the previous head and messages
 * byte for byte and only appends, with `toolChoice` `"none"`. The scorer never
 * throws; every failure resolves `fallback` with no decisions. Request
 * text, parsing, and fallbacks: `modelScorer`.
 *
 * Intentionally not in the Log: this call, and `dropped`. The compiler
 * copies usage, fallback, and `raw` onto the manifest; the kernel stores
 * that manifest.
 */

import { isRecord } from "@sage/core";
import {
  collect,
  type ModelRef,
  type ProviderAdapter,
  type Request,
} from "@sage/provider/contract";
import {
  clampSummary,
  SCORE_TIERS,
  SUMMARY_LIMIT,
} from "../compiler/score-common.js";
import { summarizeInput } from "../compiler/stub.js";
import { isTier } from "../guards.js";
import type { ScoreDecision, ScoreRaw, Scorer } from "./types.js";

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
  `summary is optional and at most ${SUMMARY_LIMIT} characters.`,
  "A summary must let a later turn locate the result without rereading it.",
  "File read: path, key symbols or exports, and the line range.",
  "grep or ls: the matching paths and line numbers.",
  'Command output: the command and conclusive numbers or status, for example "tests 23 passed 1 failed: name".',
  'Do not write a judgment only ("already reviewed", "not needed", "same as before"). Use tier none or full instead.',
  "文件读取写路径、关键符号或导出、以及行号范围；grep 或 ls 写命中路径与行号；命令输出写命令和结论性数字或状态。不要只写判断（已看过、不再需要、和之前一样），改判 none 或 full。",
  "tier none drops the result, summary keeps a short note, full keeps the original text.",
  "Example:",
  '[{"id":"seq:1#a","tier":"none"},{"id":"seq:2#b","tier":"summary","summary":"src/utils/url.ts: isURLMatched, isURLInList (L12-40)"}]',
].join("\n");

/**
 * Creates the model scorer. Its `name` is `model`.
 *
 * When `prevRequest` is missing, score resolves
 * `{ decisions: [], fallback: "no-previous-request", requested: false }`
 * and does not call an adapter; a missing adapter for `model.provider`
 * resolves `no-adapter`, likewise with `requested: false`. Otherwise the
 * request copies the previous head and messages byte for byte, keeps its
 * tools, sets `toolChoice` to `"none"`, and appends one user message: a
 * fixed instruction, the query, and a JSON list of
 * `{id, tool, summary, tokens}`. Candidates beyond `maxCandidates`
 * (default 60) drop the oldest.
 *
 * The instruction says this is a side judgment of earlier tool results,
 * not a continuation of the task, and that the reply is a JSON array of
 * `{id, tier, summary?}`. A summary must locate the result: a file read
 * names the path, key symbols, and line range; grep or ls names hit paths
 * and line numbers; command output names the command and conclusive
 * counts. A judgment-only note is not a summary. `timeoutMs` defaults to
 * 20000.
 *
 * The parser strips one fenced json block, then takes the first top-level
 * JSON value (`[` or `{`). Brackets pair, and brackets inside strings do
 * not count. Text before or after that value is ignored. It wraps a single
 * object that has `id` into a one-item array, drops items whose id is not
 * in the listed candidates (`dropped`), and treats `[]` as keep-all. A
 * malformed item fails the parse.
 *
 * Never throws. No extractable value, invalid JSON, a timeout, an abort,
 * and a finish error resolve `fallback` and no decisions. When that
 * fallback follows a model reply, `raw` is `{ text, finish }` with at most
 * 300 characters of text. A completed call, including one whose reply has
 * surrounding prose, returns the parsed decisions and the stream usage,
 * and does not set `raw`.
 */
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
        return {
          decisions: [],
          fallback: "no-previous-request",
          requested: false,
        };
      }
      const adapter = opts.adapters[opts.model.provider];
      if (!adapter) {
        return { decisions: [], fallback: "no-adapter", requested: false };
      }

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
        { api: adapter.api },
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
  const extracted = extractFirstJsonValue(stripFence(text));
  if (extracted === undefined) return undefined;
  let parsed: unknown;
  try {
    parsed = JSON.parse(extracted);
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

function extractFirstJsonValue(text: string): string | undefined {
  const start = text.search(/[[{]/);
  if (start < 0) return undefined;
  const stack: string[] = [];
  let inString = false;
  let escaped = false;
  for (let i = start; i < text.length; i++) {
    const ch = text[i];
    if (ch === undefined) return undefined;
    if (inString) {
      if (escaped) {
        escaped = false;
        continue;
      }
      if (ch === "\\") {
        escaped = true;
        continue;
      }
      if (ch === '"') inString = false;
      continue;
    }
    if (ch === '"') {
      inString = true;
      continue;
    }
    if (ch === "[") {
      stack.push("]");
      continue;
    }
    if (ch === "{") {
      stack.push("}");
      continue;
    }
    if (ch !== "]" && ch !== "}") continue;
    if (stack.at(-1) !== ch) return undefined;
    stack.pop();
    if (stack.length === 0) return text.slice(start, i + 1);
  }
  return undefined;
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
  if (!isTier(item.tier) || !SCORE_TIERS.has(item.tier)) {
    return { kind: "fail" };
  }
  if (item.summary !== undefined && typeof item.summary !== "string") {
    return { kind: "fail" };
  }
  const decision: ScoreDecision = {
    id: item.id,
    tier: item.tier,
  };
  if (typeof item.summary === "string") {
    decision.summary = clampSummary(item.summary);
  }
  return { kind: "keep", decision };
}
