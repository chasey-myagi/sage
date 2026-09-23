/**
 * Fold a chunk stream into one assistant message.
 * Tool input is the concatenation of partial JSON, parsed once at the end.
 * A parse failure keeps rawInput and sets input to null.
 * responseId and responseModel come from the response chunk. Without one
 * (the call failed before the body began) both are the empty string.
 *
 * Deliberately not a Log entry: collect does not append. The caller decides
 * whether the resulting message becomes an assistant entry.
 */

import type { AssistantMessage, ProviderAdapter } from "./adapter.js";
import type { ContentBlock } from "./message.js";
import type { StreamChunk } from "./stream.js";
import type { Usage } from "./usage.js";

type Acc = {
  index: number;
  blockType: ContentBlock["type"];
  id?: string;
  name?: string;
  text: string;
  rawInput: string;
};

const EMPTY_USAGE: Usage = {
  inputOther: 0,
  inputCacheRead: 0,
  inputCacheCreation: 0,
  output: 0,
};

export async function collect(
  chunks: AsyncIterable<StreamChunk>,
  meta: { api: ProviderAdapter["api"] },
): Promise<AssistantMessage> {
  const blocks = new Map<number, Acc>();
  let responseId = "";
  let responseModel = "";
  let usage: Usage = EMPTY_USAGE;
  let finish: Extract<StreamChunk, { type: "finish" }> = {
    type: "finish",
    kind: "error",
    error: { message: "stream ended before completion" },
  };

  for await (const chunk of chunks) {
    switch (chunk.type) {
      case "response":
        responseId = chunk.id;
        responseModel = chunk.model;
        break;
      case "block_start":
        blocks.set(chunk.index, {
          index: chunk.index,
          blockType: chunk.blockType,
          id: chunk.id,
          name: chunk.name,
          text: "",
          rawInput: "",
        });
        break;
      case "text_delta":
      case "thinking_delta":
        appendText(blocks, chunk.index, chunk.text);
        break;
      case "tool_input_delta":
        appendRaw(blocks, chunk.index, chunk.partialJson);
        break;
      case "block_end":
        break;
      case "usage":
        usage = chunk.usage;
        break;
      case "finish":
        finish = chunk;
        break;
    }
  }

  const content = [...blocks.values()]
    .sort((left, right) => left.index - right.index)
    .map(toBlock);

  return {
    role: "assistant",
    content,
    usage,
    finish,
    responseModel,
    responseId,
    api: meta.api,
  };
}

function appendText(
  blocks: Map<number, Acc>,
  index: number,
  text: string,
): void {
  const acc = blocks.get(index);
  if (!acc) return;
  acc.text += text;
}

function appendRaw(
  blocks: Map<number, Acc>,
  index: number,
  partial: string,
): void {
  const acc = blocks.get(index);
  if (!acc) return;
  acc.rawInput += partial;
}

function toBlock(acc: Acc): ContentBlock {
  switch (acc.blockType) {
    case "text":
      return { type: "text", text: acc.text };
    case "thinking":
      return { type: "thinking", text: acc.text };
    case "tool_use":
      return {
        type: "tool_use",
        id: acc.id ?? "",
        name: acc.name ?? "",
        input: parseInput(acc.rawInput),
        rawInput: acc.rawInput,
      };
    case "tool_result":
      return {
        type: "tool_result",
        toolUseId: acc.id ?? "",
        content: acc.text,
        isError: false,
      };
  }
}

function parseInput(rawInput: string): unknown {
  try {
    return JSON.parse(rawInput) as unknown;
  } catch {
    return null;
  }
}
