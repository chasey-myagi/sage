/**
 * Prefix-aware rendering of one model request, or of the compiled view it
 * is sent from (both carry the same head and messages).
 *
 * Guarantees: the text is `system:`, `tools:`, then `messages:`. Tools are
 * JSON with sorted object keys. Each message starts with `[role]` and then
 * one line per block. When `previous` has the same head and its messages are
 * a JSON prefix of this request, the render collapses to `<same>` / `<last>`
 * plus the added messages. Any other change prints the whole prompt.
 *
 * Intentionally not in the Log: this text. The request it renders is already
 * recoverable from message and request_header entries.
 */

import {
  type ContentBlock,
  isRecord,
  type Message,
  type Request,
} from "@sage/provider/contract";

/** The parts of a request (or a `CompiledView`) the render reads. */
type Rendered = Pick<Request, "head" | "messages">;

function sortedReplacer(_key: string, value: unknown): unknown {
  if (!isRecord(value)) return value;
  const sorted: Record<string, unknown> = {};
  for (const key of Object.keys(value).sort()) {
    sorted[key] = value[key];
  }
  return sorted;
}

function renderBlock(block: ContentBlock): string {
  switch (block.type) {
    case "text":
      return block.text;
    case "thinking":
      return `thinking ${block.text.length} chars`;
    case "tool_use":
      return `tool_use ${block.name} ${block.rawInput}`;
    case "tool_result":
      return `tool_result ${block.toolUseId} ${block.content}`;
  }
}

function renderMessage(message: Message): string {
  if (message.content.length === 0) return `[${message.role}]`;
  return message.content
    .map((block, index) => {
      const line = renderBlock(block);
      return index === 0 ? `[${message.role}] ${line}` : line;
    })
    .join("\n");
}

function sameHead(left: Rendered, right: Rendered): boolean {
  return (
    left.head.system === right.head.system &&
    JSON.stringify(left.head.tools) === JSON.stringify(right.head.tools)
  );
}

function isMessagePrefix(
  previous: readonly Message[],
  current: readonly Message[],
): boolean {
  if (previous.length > current.length) return false;
  return previous.every(
    (message, index) =>
      JSON.stringify(message) === JSON.stringify(current[index]),
  );
}

function fullRender(request: Rendered): string {
  const tools = JSON.stringify(request.head.tools, sortedReplacer, 2);
  const messages = request.messages.map(renderMessage).join("\n");
  const head = `system: ${request.head.system}\ntools: ${tools}\nmessages:`;
  return messages.length === 0 ? head : `${head}\n${messages}`;
}

/**
 * Renders `request`, collapsed against `previous` when it is a prefix. Takes
 * a `Request` or a `CompiledView`; only `head` and `messages` are read.
 */
export function renderLlmInput(request: Rendered, previous?: Rendered): string {
  if (
    previous === undefined ||
    !sameHead(previous, request) ||
    !isMessagePrefix(previous.messages, request.messages)
  ) {
    return fullRender(request);
  }
  const added = request.messages.slice(previous.messages.length);
  const lines = ["<last>"];
  for (const message of added) {
    for (const line of renderMessage(message).split("\n")) {
      lines.push(`+ ${line}`);
    }
  }
  return `system: <same>\ntools: <same>\nmessages:\n${lines.join("\n")}`;
}
