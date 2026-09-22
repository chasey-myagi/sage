/**
 * Anthropic Messages adapter. Translates cache breakpoints into
 * cache_control and folds SSE back into StreamChunk.
 *
 * Deliberately not a Log entry: the API key, request bytes, and abort
 * signal stay here. adapterDefaults are returned so a later
 * request_header can store what this adapter filled in.
 */

import { prepareModel, resolveMaxTokens } from "../catalog.js";
import type { PreparedCall, ProviderAdapter } from "../contract/adapter.js";
import type {
  CacheBreakpoint,
  ContentBlock,
  ModelRef,
  Request,
} from "../contract/message.js";
import type { StreamChunk } from "../contract/stream.js";
import type { Usage } from "../contract/usage.js";
import type { Driver, WireStop } from "../drive.js";
import { blockStart, driveStream } from "../drive.js";
import type { ClientOptions } from "../http.js";
import { joinUrl } from "../http.js";
import { asRecord, indexOf, num } from "../record.js";
import type { SseEvent } from "../sse.js";

const DEFAULT_BASE = "https://api.anthropic.com";

type Trace = { responseId: string; responseModel: string };

type AnthropicState = {
  rawUsage: Record<string, unknown>;
  stopReason: string | null;
};

export class AnthropicAdapter implements ProviderAdapter {
  readonly api = "anthropic-messages" as const;
  private readonly apiKey: string;
  private readonly baseUrl: string;
  private readonly fetchImpl: typeof fetch | undefined;

  constructor(options: ClientOptions) {
    this.apiKey = options.apiKey;
    this.baseUrl = options.baseUrl ?? DEFAULT_BASE;
    this.fetchImpl = options.fetchImpl;
  }

  prepare(model: ModelRef): PreparedCall {
    return prepareModel(model);
  }

  stream(
    request: Request,
    prepared: PreparedCall,
    signal?: AbortSignal,
  ): AsyncIterable<StreamChunk> {
    const responseTrace: Trace = { responseId: "", responseModel: "" };
    const iterable: AsyncIterable<StreamChunk> & { responseTrace: Trace } = {
      responseTrace,
      [Symbol.asyncIterator]: (): AsyncIterator<StreamChunk> => {
        const state: AnthropicState = { rawUsage: {}, stopReason: null };
        return driveStream({
          url: joinUrl(this.baseUrl, "/v1/messages"),
          headers: {
            "content-type": "application/json",
            "x-api-key": this.apiKey,
            "anthropic-version": "2023-06-01",
          },
          json: encodeRequest(request, prepared),
          signal,
          fetchImpl: this.fetchImpl,
          onEvent(event, driver) {
            interpret(event, responseTrace, state, driver);
          },
        });
      },
    };
    return iterable;
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
  const messages = request.messages.map((message, index) => ({
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
        input: block.input,
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
  trace: Trace,
  state: AnthropicState,
  driver: Driver,
): void {
  let payload: unknown;
  try {
    payload = JSON.parse(event.data);
  } catch {
    return;
  }
  const record = asRecord(payload);
  if (!record || typeof record.type !== "string") return;
  switch (record.type) {
    case "message_start":
      onMessageStart(record, trace, state, driver);
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
    default:
      return;
  }
}

function onMessageStart(
  record: Record<string, unknown>,
  trace: Trace,
  state: AnthropicState,
  driver: Driver,
): void {
  const message = asRecord(record.message);
  if (!message) return;
  if (typeof message.id === "string") trace.responseId = message.id;
  if (typeof message.model === "string") trace.responseModel = message.model;
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
    inputOther: num(raw.input_tokens),
    inputCacheRead: num(raw.cache_read_input_tokens),
    inputCacheCreation: num(raw.cache_creation_input_tokens),
    output: num(raw.output_tokens),
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
