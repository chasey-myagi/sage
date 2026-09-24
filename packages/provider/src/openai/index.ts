/**
 * OpenAI-compatible chat adapter.
 *
 * Guarantees: cacheBreakpoints are ignored, since this wire format has no
 * cache field. Thinking blocks and foreign tool ids are normalized before
 * encoding; thinking is never sent back. Streamed `reasoning_content`
 * opens its own thinking block. The first parsed SSE object's `id` and
 * `model` become the response chunk. `toolChoice` is sent as
 * `tool_choice` unchanged; an absent toolChoice omits the field. Messages
 * and tools are otherwise sent unchanged.
 *
 * Intentionally not in the Log: the API key, request bytes, and abort
 * signal. adapterDefaults are returned so the request_header can store what
 * this adapter filled in. Pre-stream retries are not adapter defaults and
 * not a request_header field.
 */

import { prepareModel, resolveMaxTokens } from "../catalog.js";
import type { PreparedCall, ProviderAdapter } from "../contract/adapter.js";
import type { Message, ModelRef, Request } from "../contract/message.js";
import type { StreamChunk } from "../contract/stream.js";
import type { Usage } from "../contract/usage.js";
import type { Driver, WireStop } from "../drive.js";
import { blockStart, driveStream } from "../drive.js";
import { type Client, type ClientOptions, resolveClient } from "../http.js";
import { normalizeForApi } from "../normalize.js";
import { asRecord, indexOf, num, parseRecord } from "../record.js";
import type { SseEvent } from "../sse.js";

const DEFAULT_BASE = "https://api.openai.com/v1";

type OpenKind = "text" | "tool" | "thinking";

type OpenState = {
  nextIndex: number;
  open?: { index: number; kind: OpenKind };
  toolBlocks: Map<number, number>;
  finishReason?: string;
  /** Set once the response chunk has been pushed. */
  announced: boolean;
};

/**
 * OpenAI-compatible chat adapter. Differs from `AnthropicAdapter` only where
 * the protocol does: the `/chat/completions` path, bearer auth, the request
 * encoding, and the event decoder (which must allocate block indices itself,
 * since this wire format has none).
 */
export class OpenAIAdapter implements ProviderAdapter {
  readonly api = "openai-chat" as const;
  private readonly client: Client;

  constructor(options: ClientOptions) {
    this.client = resolveClient(options, DEFAULT_BASE);
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
    return driveStream(this.client, {
      path: "/chat/completions",
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${this.client.apiKey}`,
      },
      json: encodeRequest(request, prepared),
      signal,
      onEvent: (event, driver) => interpret(event, state, driver),
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
  const record = parseRecord(event.data);
  if (!record) return;
  const choice = firstRecord(record.choices);
  // Announce on the first frame that names the response. A frame with a
  // choice may open a block, so announce before it even without an id: the
  // response chunk must precede every block_start.
  if (!state.announced && (typeof record.id === "string" || choice)) {
    state.announced = true;
    driver.push({
      type: "response",
      id: typeof record.id === "string" ? record.id : "",
      model: typeof record.model === "string" ? record.model : "",
    });
  }
  if (choice) {
    const delta = asRecord(choice.delta);
    if (delta && typeof delta.reasoning_content === "string") {
      applyText(state, driver, "thinking", delta.reasoning_content);
    }
    if (delta && typeof delta.content === "string") {
      applyText(state, driver, "text", delta.content);
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

/**
 * Appends streamed text of one kind. A different open block is closed first
 * and a new block opened. An empty delta only opens a block when none is
 * open; it never closes one.
 */
function applyText(
  state: OpenState,
  driver: Driver,
  kind: "text" | "thinking",
  text: string,
): void {
  if (text.length === 0) {
    if (!state.open) openBlock(state, driver, kind);
    return;
  }
  if (state.open?.kind !== kind) closeOpen(state, driver);
  const index = state.open?.index ?? openBlock(state, driver, kind);
  driver.push(
    kind === "text"
      ? { type: "text_delta", index, text }
      : { type: "thinking_delta", index, text },
  );
}

/** Opens the next block index as `kind` and returns that index. */
function openBlock(
  state: OpenState,
  driver: Driver,
  kind: OpenKind,
  id?: string,
  name?: string,
): number {
  const index = state.nextIndex;
  state.nextIndex += 1;
  state.open = { index, kind };
  driver.push(blockStart(index, kind === "tool" ? "tool_use" : kind, id, name));
  return index;
}

/**
 * Tool call deltas are keyed by the wire's `index`; the first delta for an
 * index closes any open block and opens a tool_use block.
 */
function applyTool(state: OpenState, driver: Driver, value: unknown): void {
  const call = asRecord(value);
  if (!call) return;
  const toolIndex = indexOf(call.index);
  if (toolIndex === undefined) return;
  const fn = asRecord(call.function);
  let blockIndex = state.toolBlocks.get(toolIndex);
  if (blockIndex === undefined) {
    closeOpen(state, driver);
    const id = typeof call.id === "string" ? call.id : undefined;
    const name = fn && typeof fn.name === "string" ? fn.name : undefined;
    blockIndex = openBlock(state, driver, "tool", id, name);
    state.toolBlocks.set(toolIndex, blockIndex);
  }
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

/**
 * Ends the open block, if any. Called when a block of another kind starts,
 * when the choice reports a finish_reason, and at `[DONE]`.
 */
function closeOpen(state: OpenState, driver: Driver): void {
  if (!state.open) return;
  driver.push({ type: "block_end", index: state.open.index });
  state.open = undefined;
}

function firstRecord(value: unknown): Record<string, unknown> | undefined {
  if (!Array.isArray(value)) return undefined;
  return asRecord(value[0]);
}

/**
 * Cache reads come from `prompt_cache_hit_tokens` (DeepSeek), then
 * `prompt_tokens_details.cached_tokens`, else none; the rest of
 * `prompt_tokens` is `inputOther`. This protocol reports no cache writes,
 * so `inputCacheCreation` is 0. `raw` is the provider object unchanged.
 */
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
