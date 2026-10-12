/**
 * Anthropic Messages adapter.
 *
 * Guarantees: cache breakpoints become `cache_control` marks (the head mark
 * goes on the system block and the last tool). `message_start` becomes the
 * response chunk (message.id, message.model). `toolChoice` becomes
 * `tool_choice: { type }`; an absent toolChoice omits the field. A
 * tool_use replays its `input`, or `{}` when that is not a JSON object
 * (`replayToolInput`, a pure function of the logged block). Messages and
 * tools are otherwise sent unchanged. An `error` event ends the call
 * as a non-transport error finish carrying the server's message and
 * `error.event`; its `error.type` is permanent when it names a request the
 * API will reject again.
 *
 * Intentionally not in the Log: the API key, request bytes, and abort
 * signal. adapterDefaults are returned so the request_header can store what
 * this adapter filled in. Pre-stream retries are not adapter defaults and
 * not a request_header field.
 */

import {
  type AdapterOptions,
  type CatalogEntry,
  prepareModel,
  resolveMaxTokens,
} from "../catalog.js";
import type { PreparedCall, ProviderAdapter } from "../contract/adapter.js";
import type {
  CacheBreakpoint,
  ContentBlock,
  ModelRef,
  Request,
} from "../contract/message.js";
import { finiteOr } from "../contract/record.js";
import type { StreamChunk } from "../contract/stream.js";
import { replayToolInput } from "../contract/tool-input.js";
import type { Usage } from "../contract/usage.js";
import type { Driver, WireStop } from "../drive.js";
import { blockStart, driveStream } from "../drive.js";
import { type Client, resolveClient } from "../http.js";
import { normalizeForApi } from "../normalize.js";
import { asRecord, indexOf, parseRecord } from "../record.js";
import type { SseEvent } from "../sse.js";

const DEFAULT_BASE = "https://api.anthropic.com";
/** Messages API version header this encoder and decoder are written against. */
const ANTHROPIC_VERSION = "2023-06-01";
/**
 * Error event types a resend cannot fix. `overloaded_error`, `api_error`,
 * `rate_limit_error`, and unknown types stay transient.
 */
const PERMANENT_ERRORS: ReadonlySet<string> = new Set([
  "invalid_request_error",
  "authentication_error",
  "permission_error",
  "not_found_error",
  "request_too_large",
  "billing_error",
]);

type AnthropicState = {
  rawUsage: Record<string, unknown>;
  stopReason: string | null;
};

/**
 * Anthropic Messages adapter. Differs from `OpenAIAdapter` only where the
 * protocol does: the `/v1/messages` path, `x-api-key` plus
 * `anthropic-version` headers, the request encoding, and the event decoder.
 */
export class AnthropicAdapter implements ProviderAdapter {
  readonly api = "anthropic-messages" as const;
  private readonly client: Client;
  private readonly models: readonly CatalogEntry[];

  constructor(options: AdapterOptions) {
    const { models = [], ...transport } = options;
    this.client = resolveClient(transport, DEFAULT_BASE);
    this.models = models;
  }

  prepare(model: ModelRef): PreparedCall {
    return prepareModel(model, this.models);
  }

  stream(
    request: Request,
    prepared: PreparedCall,
    signal?: AbortSignal,
  ): AsyncIterable<StreamChunk> {
    const state: AnthropicState = { rawUsage: {}, stopReason: null };
    return driveStream(this.client, {
      path: "/v1/messages",
      headers: {
        "content-type": "application/json",
        "x-api-key": this.client.apiKey,
        "anthropic-version": ANTHROPIC_VERSION,
      },
      json: encodeRequest(request, prepared),
      signal,
      onEvent: (event, driver) => interpret(event, state, driver),
      permanentErrors: PERMANENT_ERRORS,
    });
  }
}

function encodeRequest(
  request: Request,
  prepared: PreparedCall,
): Record<string, unknown> {
  const cache = cachePlan(request.cacheBreakpoints);
  const system = [
    {
      type: "text",
      text: request.head.system,
      ...(cache.head ? { cache_control: { type: "ephemeral" } } : {}),
    },
  ];
  const tools = request.head.tools.map((tool, index) => ({
    name: tool.name,
    description: tool.description,
    input_schema: tool.inputSchema,
    ...(cache.head && index === request.head.tools.length - 1
      ? { cache_control: { type: "ephemeral" } }
      : {}),
  }));
  const normalized = normalizeForApi(request.messages, "anthropic-messages");
  const messages = normalized.map((message, index) => ({
    role: message.role,
    content: message.content.map((block, blockIndex) =>
      encodeBlock(
        block,
        cache.after.has(index) && blockIndex === message.content.length - 1,
      ),
    ),
  }));
  const body: Record<string, unknown> = {
    model: request.model.model,
    max_tokens: resolveMaxTokens(request, prepared),
    stream: true,
    system,
    messages,
  };
  if (request.sampling.temperature !== undefined) {
    body.temperature = request.sampling.temperature;
  }
  if (tools.length > 0) body.tools = tools;
  if (request.toolChoice !== undefined) {
    body.tool_choice = { type: request.toolChoice };
  }
  return body;
}

function cachePlan(breakpoints: CacheBreakpoint[]): {
  head: boolean;
  after: Set<number>;
} {
  const after = new Set<number>();
  let head = false;
  for (const breakpoint of breakpoints) {
    if (breakpoint === "head") head = true;
    else after.add(breakpoint.afterMessage);
  }
  return { head, after };
}

function encodeBlock(
  block: ContentBlock,
  cache: boolean,
): Record<string, unknown> {
  const mark = cache ? { cache_control: { type: "ephemeral" } } : {};
  switch (block.type) {
    case "text":
      return { type: "text", text: block.text, ...mark };
    case "thinking":
      if (block.redacted) {
        return {
          type: "redacted_thinking",
          data: block.signature ?? "",
          ...mark,
        };
      }
      return {
        type: "thinking",
        thinking: block.text,
        ...(block.signature !== undefined
          ? { signature: block.signature }
          : {}),
        ...mark,
      };
    case "tool_use":
      return {
        type: "tool_use",
        id: block.id,
        name: block.name,
        input: replayToolInput(block).input,
        ...mark,
      };
    case "tool_result":
      return {
        type: "tool_result",
        tool_use_id: block.toolUseId,
        content: block.content,
        is_error: block.isError,
        ...mark,
      };
  }
}

function interpret(
  event: SseEvent,
  state: AnthropicState,
  driver: Driver,
): void {
  const record = parseRecord(event.data);
  if (!record || typeof record.type !== "string") return;
  switch (record.type) {
    case "message_start":
      onMessageStart(record, state, driver);
      return;
    case "content_block_start":
      onBlockStart(record, driver);
      return;
    case "content_block_delta":
      onBlockDelta(record, driver);
      return;
    case "content_block_stop":
      onBlockStop(record, driver);
      return;
    case "message_delta":
      onMessageDelta(record, state, driver);
      return;
    case "message_stop":
      driver.complete(mapStop(state.stopReason));
      return;
    case "error":
      driver.fail(record);
      return;
    default:
      return;
  }
}

function onMessageStart(
  record: Record<string, unknown>,
  state: AnthropicState,
  driver: Driver,
): void {
  const message = asRecord(record.message);
  if (!message) return;
  driver.push({
    type: "response",
    id: typeof message.id === "string" ? message.id : "",
    model: typeof message.model === "string" ? message.model : "",
  });
  const usage = asRecord(message.usage);
  if (!usage) return;
  state.rawUsage = { ...usage };
  driver.setUsage(mapUsage(state.rawUsage));
}

function onBlockStart(record: Record<string, unknown>, driver: Driver): void {
  const index = indexOf(record.index);
  const block = asRecord(record.content_block);
  if (index === undefined || !block || typeof block.type !== "string") return;
  const blockType = wireBlockType(block.type);
  if (!blockType) return;
  const id = typeof block.id === "string" ? block.id : undefined;
  const name = typeof block.name === "string" ? block.name : undefined;
  driver.push(blockStart(index, blockType, id, name));
}

function onBlockDelta(record: Record<string, unknown>, driver: Driver): void {
  const index = indexOf(record.index);
  const delta = asRecord(record.delta);
  if (index === undefined || !delta || typeof delta.type !== "string") return;
  if (delta.type === "text_delta" && typeof delta.text === "string") {
    driver.push({ type: "text_delta", index, text: delta.text });
    return;
  }
  if (delta.type === "thinking_delta" && typeof delta.thinking === "string") {
    driver.push({ type: "thinking_delta", index, text: delta.thinking });
    return;
  }
  if (
    delta.type === "input_json_delta" &&
    typeof delta.partial_json === "string"
  ) {
    driver.push({
      type: "tool_input_delta",
      index,
      partialJson: delta.partial_json,
    });
  }
}

function onBlockStop(record: Record<string, unknown>, driver: Driver): void {
  const index = indexOf(record.index);
  if (index === undefined) return;
  driver.push({ type: "block_end", index });
}

function onMessageDelta(
  record: Record<string, unknown>,
  state: AnthropicState,
  driver: Driver,
): void {
  const delta = asRecord(record.delta);
  if (delta && "stop_reason" in delta) {
    const reason = delta.stop_reason;
    if (typeof reason === "string") state.stopReason = reason;
    else if (reason === null) state.stopReason = null;
  }
  const usage = asRecord(record.usage);
  if (!usage) return;
  state.rawUsage = { ...state.rawUsage, ...usage };
  driver.setUsage(mapUsage(state.rawUsage));
}

function wireBlockType(type: string): ContentBlock["type"] | undefined {
  if (type === "text") return "text";
  if (type === "tool_use") return "tool_use";
  if (type === "thinking" || type === "redacted_thinking") return "thinking";
  return undefined;
}

function mapUsage(raw: Record<string, unknown>): Usage {
  return {
    inputOther: finiteOr(raw.input_tokens, 0),
    inputCacheRead: finiteOr(raw.cache_read_input_tokens, 0),
    inputCacheCreation: finiteOr(raw.cache_creation_input_tokens, 0),
    output: finiteOr(raw.output_tokens, 0),
    raw,
  };
}

function mapStop(reason: string | null): WireStop {
  switch (reason) {
    case "end_turn":
    case null:
      return { kind: "stop" };
    case "tool_use":
      return { kind: "tool_use" };
    case "max_tokens":
      return { kind: "max_tokens" };
    default:
      return { kind: "stop", rawFinishReason: reason };
  }
}
