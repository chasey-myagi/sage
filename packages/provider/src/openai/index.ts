/**
 * OpenAI-compatible chat adapter. cacheBreakpoints are ignored: this wire
 * format has no cache field. Thinking blocks are dropped; cross-provider
 * downgrade belongs to #13.
 *
 * Deliberately not a Log entry: the API key, request bytes, and abort
 * signal stay here. adapterDefaults are returned so a later
 * request_header can store what this adapter filled in.
 */

import { prepareModel, resolveMaxTokens } from "../catalog.js";
import type { PreparedCall, ProviderAdapter } from "../contract/adapter.js";
import type { Message, ModelRef, Request } from "../contract/message.js";
import type { StreamChunk } from "../contract/stream.js";
import type { Usage } from "../contract/usage.js";
import type { Driver, WireStop } from "../drive.js";
import { blockStart, driveStream } from "../drive.js";
import type { ClientOptions } from "../http.js";
import { joinUrl } from "../http.js";
import { asRecord, indexOf, num } from "../record.js";
import type { SseEvent } from "../sse.js";

const DEFAULT_BASE = "https://api.openai.com/v1";

type Trace = { responseId: string; responseModel: string };

type OpenState = {
  nextIndex: number;
  open?: { index: number; kind: "text" | "tool" };
  toolBlocks: Map<number, number>;
  finishReason?: string;
};

export class OpenAIAdapter implements ProviderAdapter {
  readonly api = "openai-chat" as const;
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
        const state: OpenState = { nextIndex: 0, toolBlocks: new Map() };
        return driveStream({
          url: joinUrl(this.baseUrl, "/chat/completions"),
          headers: {
            "content-type": "application/json",
            authorization: `Bearer ${this.apiKey}`,
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
  const messages: Record<string, unknown>[] = [
    { role: "system", content: request.head.system },
  ];
  for (const message of request.messages) {
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

function interpret(
  event: SseEvent,
  trace: Trace,
  state: OpenState,
  driver: Driver,
): void {
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
  if (typeof record.id === "string") trace.responseId = record.id;
  if (typeof record.model === "string") trace.responseModel = record.model;
  const choice = firstRecord(record.choices);
  if (choice) {
    const delta = asRecord(choice.delta);
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

function applyContent(state: OpenState, driver: Driver, content: string): void {
  if (content.length === 0) {
    if (!state.open) openText(state, driver);
    return;
  }
  if (state.open?.kind === "tool") closeOpen(state, driver);
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
  const details = asRecord(raw.prompt_tokens_details);
  const cached = details ? num(details.cached_tokens) : 0;
  return {
    inputOther: num(raw.prompt_tokens) - cached,
    inputCacheRead: cached,
    inputCacheCreation: 0,
    output: num(raw.completion_tokens),
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
