/**
 * Cross-provider message normalization. Pure function.
 *
 * Contract: `openai-chat` drops every thinking block. Tool ids that start
 * with `toolu_` become `call_` plus the first 24 hex chars of sha256(id);
 * matching tool_result ids change with them. Other ids stay. `anthropic-messages`
 * drops thinking blocks that have no signature and leaves every other block
 * unchanged, including tool ids.
 *
 * Intentionally not in the Log: this function. Adapters run it on a copy
 * while encoding. The kernel logs the messages before that copy.
 */

import { createHash } from "node:crypto";
import type { ProviderAdapter } from "./contract/adapter.js";
import type { ContentBlock, Message } from "./contract/message.js";

export function normalizeForApi(
  messages: Message[],
  api: ProviderAdapter["api"],
): Message[] {
  if (api === "openai-chat") return forOpenAI(messages);
  return forAnthropic(messages);
}

function forOpenAI(messages: readonly Message[]): Message[] {
  const ids = new Map<string, string>();
  for (const message of messages) {
    for (const block of message.content) {
      if (block.type === "tool_use" && block.id.startsWith("toolu_")) {
        ids.set(block.id, openAiToolId(block.id));
      }
    }
  }
  return messages.map((message) => ({
    role: message.role,
    content: message.content.flatMap((block) => mapOpenAI(block, ids)),
  }));
}

function mapOpenAI(
  block: ContentBlock,
  ids: ReadonlyMap<string, string>,
): ContentBlock[] {
  if (block.type === "thinking") return [];
  if (block.type === "tool_use") {
    const id = ids.get(block.id) ?? block.id;
    return id === block.id ? [block] : [{ ...block, id }];
  }
  if (block.type === "tool_result") {
    const toolUseId = ids.get(block.toolUseId) ?? block.toolUseId;
    return toolUseId === block.toolUseId ? [block] : [{ ...block, toolUseId }];
  }
  return [block];
}

function forAnthropic(messages: readonly Message[]): Message[] {
  return messages.map((message) => ({
    role: message.role,
    content: message.content.filter((block) => {
      if (block.type !== "thinking") return true;
      return typeof block.signature === "string" && block.signature.length > 0;
    }),
  }));
}

function openAiToolId(id: string): string {
  const hex = createHash("sha256").update(id).digest("hex").slice(0, 24);
  return `call_${hex}`;
}
