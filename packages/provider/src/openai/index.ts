/**
 * OpenAI-compatible chat adapter. cacheBreakpoints are ignored: this wire
 * format has no cache field. Thinking blocks and foreign tool ids are
 * normalized before encode. SSE `delta.reasoning_content` string increments
 * open their own thinking block and emit thinking_delta. block_end fires
 * when content starts or the choice finishes. Encode still drops thinking
 * through normalizeForApi and does not send reasoning_content back. Usage
 * prefers prompt_cache_hit_tokens, then prompt_tokens_details.cached_tokens,
 * then the whole prompt_tokens. inputCacheCreation stays 0. raw is the
 * provider usage object unchanged. The first parsed SSE object's `id` and
 * `model` become the response chunk. `toolChoice: "none"` is
 * `tool_choice: "none"` and `"auto"` is `"auto"`. An absent toolChoice
 * omits the field. messages and tools are otherwise unchanged.
 *
 * Deliberately not a Log entry: the API key, request bytes, and abort
 * signal stay here. adapterDefaults are returned so a later
 * request_header can store what this adapter filled in. Pre-stream
 * retries are not adapter defaults and are not a request_header field.
 */

import { prepareModel, resolveMaxTokens } from "../catalog.js";
import type { PreparedCall, ProviderAdapter } from "../contract/adapter.js";
import type { Sleeper } from "../contract/backoff.js";
import type { Message, ModelRef, Request } from "../contract/message.js";
import type { StreamChunk } from "../contract/stream.js";
import type { Usage } from "../contract/usage.js";
import type { Driver, WireStop } from "../drive.js";
import { blockStart, driveStream } from "../drive.js";
import type { ClientOptions } from "../http.js";
import { joinUrl } from "../http.js";
import { normalizeForApi } from "../normalize.js";
import { asRecord, indexOf, num } from "../record.js";
import type { RetryPolicy } from "../retry.js";
import type { SseEvent } from "../sse.js";

const DEFAULT_BASE = "https://api.openai.com/v1";

type OpenState = {
  nextIndex: number;
  open?: { index: number; kind: "text" | "tool" | "thinking" };
  toolBlocks: Map<number, number>;
  finishReason?: string;
  /** Set once the response chunk has been pushed. */
  announced: boolean;
};

export class OpenAIAdapter implements ProviderAdapter {
  readonly api = "openai-chat" as const;
  private readonly apiKey: string;
  private readonly baseUrl: string;
  private readonly fetchImpl: typeof fetch | undefined;
  private readonly retry: RetryPolicy | undefined;
  private readonly sleep: Sleeper | undefined;

  constructor(options: ClientOptions) {
    this.apiKey = options.apiKey;
    this.baseUrl = options.baseUrl ?? DEFAULT_BASE;
    this.fetchImpl = options.fetchImpl;
    this.retry = options.retry;
    this.sleep = options.sleep;
  }

  prepare(model: ModelRef): PreparedCall {
    return prepareModel(model);
  }

  stream(
    request: Request,
    prepared: PreparedCall,
    signal?: AbortSignal,
  ): AsyncIterable<StreamChunk> {
    const state: OpenState = {
      nextIndex: 0,
      toolBlocks: new Map(),
      announced: false,
    };
    return driveStream({
      url: joinUrl(this.baseUrl, "/chat/completions"),
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${this.apiKey}`,
      },
      json: encodeRequest(request, prepared),
      signal,
      fetchImpl: this.fetchImpl,
      retry: this.retry,
      sleep: this.sleep,
      onEvent(event, driver) {
        interpret(event, state, driver);
      },
    });
  }
}

function encodeRequest(
  request: Request,
  prepared: PreparedCall,
): Record<string, unknown> {
  const messages: Record<string, unknown>[] = [
    { role: "system", content: request.head.system },
  ];
  const normalized = normalizeForApi(request.messages, "openai-chat");
  for (const message of normalized) {
    if (message.role === "assistant") pushAssistant(messages, message);
    else pushUser(messages, message);
  }
  const tools = request.head.tools.map((tool) => ({
    type: "function",
    function: {
      name: tool.name,
      description: tool.description,
      parameters: tool.inputSchema,
    },
  }));
  const body: Record<string, unknown> = {
    model: request.model.model,
    stream: true,
    stream_options: { include_usage: true },
    max_tokens: resolveMaxTokens(request, prepared),
    messages,
  };
  if (request.sampling.temperature !== undefined) {
    body.temperature = request.sampling.temperature;
  }
  if (tools.length > 0) body.tools = tools;
  if (request.toolChoice !== undefined) body.tool_choice = request.toolChoice;
  return body;
}

function pushAssistant(
  messages: Record<string, unknown>[],
  message: Message,
): void {
  const texts: string[] = [];
  const toolCalls: Record<string, unknown>[] = [];
  for (const block of message.content) {
    if (block.type === "thinking") continue;
    if (block.type === "text") texts.push(block.text);
    else if (block.type === "tool_use") {
      toolCalls.push({
        id: block.id,
        type: "function",
        function: { name: block.name, arguments: block.rawInput },
      });
    }
  }
  if (texts.length === 0 && toolCalls.length === 0) return;
  const entry: Record<string, unknown> = {
    role: "assistant",
    content: texts.length > 0 ? texts.join("") : null,
  };
  if (toolCalls.length > 0) entry.tool_calls = toolCalls;
  messages.push(entry);
}

function pushUser(messages: Record<string, unknown>[], message: Message): void {
  let text = "";
  const flush = (): void => {
    if (text.length === 0) return;
    messages.push({ role: "user", content: text });
    text = "";
  };
  for (const block of message.content) {
    if (block.type === "text") text += block.text;
    else if (block.type === "tool_result") {
      flush();
      messages.push({
        role: "tool",
        tool_call_id: block.toolUseId,
        content: block.content,
      });
    }
  }
  flush();
}

function interpret(event: SseEvent, state: OpenState, driver: Driver): void {
  if (event.data.trim() === "[DONE]") {
    closeOpen(state, driver);
    driver.complete(mapStop(state.finishReason));
    return;
  }
  let payload: unknown;
  try {
    payload = JSON.parse(event.data);
  } catch {
    return;
  }
  const record = asRecord(payload);
  if (!record) return;
  if (!state.announced) {
    state.announced = true;
    driver.push({
      type: "response",
      id: typeof record.id === "string" ? record.id : "",
      model: typeof record.model === "string" ? record.model : "",
    });
  }
  const choice = firstRecord(record.choices);
  if (choice) {
    const delta = asRecord(choice.delta);
    if (delta && typeof delta.reasoning_content === "string") {
      applyReasoning(state, driver, delta.reasoning_content);
    }
    if (delta && typeof delta.content === "string") {
      applyContent(state, driver, delta.content);
    }
    if (delta && Array.isArray(delta.tool_calls)) {
      for (const call of delta.tool_calls) applyTool(state, driver, call);
    }
    if (typeof choice.finish_reason === "string") {
      state.finishReason = choice.finish_reason;
      closeOpen(state, driver);
    }
  }
  const usage = asRecord(record.usage);
  if (usage) driver.setUsage(mapUsage(usage));
}

function applyReasoning(state: OpenState, driver: Driver, text: string): void {
  if (text.length === 0) {
    if (!state.open) openThinking(state, driver);
    return;
  }
  if (state.open?.kind !== "thinking") closeOpen(state, driver);
  if (!state.open) openThinking(state, driver);
  if (!state.open) return;
  driver.push({
    type: "thinking_delta",
    index: state.open.index,
    text,
  });
}

function openThinking(state: OpenState, driver: Driver): void {
  const index = state.nextIndex;
  state.nextIndex += 1;
  state.open = { index, kind: "thinking" };
  driver.push(blockStart(index, "thinking"));
}

function applyContent(state: OpenState, driver: Driver, content: string): void {
  if (content.length === 0) {
    if (!state.open) openText(state, driver);
    return;
  }
  if (state.open?.kind !== "text") closeOpen(state, driver);
  if (!state.open) openText(state, driver);
  if (!state.open) return;
  driver.push({ type: "text_delta", index: state.open.index, text: content });
}

function openText(state: OpenState, driver: Driver): void {
  const index = state.nextIndex;
  state.nextIndex += 1;
  state.open = { index, kind: "text" };
  driver.push(blockStart(index, "text"));
}

function applyTool(state: OpenState, driver: Driver, value: unknown): void {
  const call = asRecord(value);
  if (!call) return;
  const toolIndex = indexOf(call.index);
  if (toolIndex === undefined) return;
  let blockIndex = state.toolBlocks.get(toolIndex);
  if (blockIndex === undefined) {
    closeOpen(state, driver);
    blockIndex = state.nextIndex;
    state.nextIndex += 1;
    state.toolBlocks.set(toolIndex, blockIndex);
    state.open = { index: blockIndex, kind: "tool" };
    const fn = asRecord(call.function);
    const id = typeof call.id === "string" ? call.id : undefined;
    const name = fn && typeof fn.name === "string" ? fn.name : undefined;
    driver.push(blockStart(blockIndex, "tool_use", id, name));
  }
  const fn = asRecord(call.function);
  const args =
    fn && typeof fn.arguments === "string" ? fn.arguments : undefined;
  if (args !== undefined && args.length > 0) {
    driver.push({
      type: "tool_input_delta",
      index: blockIndex,
      partialJson: args,
    });
  }
}

function closeOpen(state: OpenState, driver: Driver): void {
  if (!state.open) return;
  driver.push({ type: "block_end", index: state.open.index });
  state.open = undefined;
}

function firstRecord(value: unknown): Record<string, unknown> | undefined {
  if (!Array.isArray(value)) return undefined;
  return asRecord(value[0]);
}

function mapUsage(raw: Record<string, unknown>): Usage {
  const output = num(raw.completion_tokens);
  if ("prompt_cache_hit_tokens" in raw) {
    const hit = num(raw.prompt_cache_hit_tokens);
    const miss =
      "prompt_cache_miss_tokens" in raw
        ? num(raw.prompt_cache_miss_tokens)
        : num(raw.prompt_tokens) - hit;
    return {
      inputOther: miss,
      inputCacheRead: hit,
      inputCacheCreation: 0,
      output,
      raw,
    };
  }
  const details = asRecord(raw.prompt_tokens_details);
  if (details && "cached_tokens" in details) {
    const cached = num(details.cached_tokens);
    return {
      inputOther: num(raw.prompt_tokens) - cached,
      inputCacheRead: cached,
      inputCacheCreation: 0,
      output,
      raw,
    };
  }
  return {
    inputOther: num(raw.prompt_tokens),
    inputCacheRead: 0,
    inputCacheCreation: 0,
    output,
    raw,
  };
}

function mapStop(reason: string | undefined): WireStop {
  switch (reason) {
    case "stop":
    case undefined:
      return { kind: "stop" };
    case "tool_calls":
      return { kind: "tool_use" };
    case "length":
      return { kind: "max_tokens" };
    default:
      return { kind: "stop", rawFinishReason: reason };
  }
}
