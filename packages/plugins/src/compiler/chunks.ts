/**
 * Chunk index over a projected history.
 *
 * Contract: one tool_use plus its tool_result is one chunk, id
 * `seq:<assistantSeq>#<toolUseId>`. A prose message is one chunk, id
 * `seq:<seq>`, kind `user` or `assistant`. Prose that shares a message with
 * tool calls is its own chunk, emitted before those tool chunks. A user
 * message that contains only tool_result blocks is not a chunk. Tool tokens
 * are count(rawInput) plus count(result content). Prose tokens are count of
 * the text and thinking blocks joined in order. Access lists come from the
 * named tool's `accesses(input)`.
 * `canSplitAfter` is the shared boundary predicate: false after a user
 * message, false after an assistant message that contains tool_use, false
 * when the next message is a user message that contains tool_result, and
 * true otherwise.
 *
 * Intentionally not in the Log: this index. The compiler copies id, tier,
 * reason, and tokens onto the manifest entry.
 */

import type { Access, HarnessTool } from "@sage/core";
import type { Message, ToolUseBlock } from "@sage/provider/contract";
import type { TokenCounter } from "./budget.js";

/**
 * Prefix a subtask compile puts on the child's own chunk ids, keeping them
 * apart from the parent's ids. `expand` strips it to resolve the id on the
 * child's path.
 */
export const OWN_CHUNK_PREFIX = "own:";

const TOOL_CHUNK_ID = /^seq:(-?\d+)#([\s\S]+)$/;

/**
 * Parses a tool chunk id `seq:<seq>#<toolUseId>`. Returns undefined for a
 * prose chunk id, an id with `OWN_CHUNK_PREFIX`, or anything else.
 */
export function parseToolChunkId(
  id: string,
): { seq: number; toolUseId: string } | undefined {
  const match = TOOL_CHUNK_ID.exec(id);
  if (!match) return undefined;
  return { seq: Number(match[1]), toolUseId: match[2] ?? "" };
}

/**
 * Returns the toolUseId a manifest chunk id names: a tool chunk id with or
 * without `OWN_CHUNK_PREFIX`. Returns undefined for a prose chunk id and for
 * an `expand` reference such as `child:<sessionId>#<chunkId>`, which is not
 * a chunk id.
 */
export function toolUseIdOfChunk(chunkId: string): string | undefined {
  const local = chunkId.startsWith(OWN_CHUNK_PREFIX)
    ? chunkId.slice(OWN_CHUNK_PREFIX.length)
    : chunkId;
  return parseToolChunkId(local)?.toolUseId;
}

/**
 * A chunk is the unit the compiler schedules: one tool call together with
 * its result, or one prose message. A chunk enters the view whole, at one
 * tier, or not at all.
 */
export type Chunk = {
  id: string;
  kind: "user" | "assistant" | "tool";
  messageIndex: number;
  toolUseId?: string;
  toolName?: string;
  input?: unknown;
  isError?: boolean;
  tokens: number;
  accesses: Access[];
};

/**
 * Indexes projected messages into chunks, in message order. `seqs[i]` is
 * the Log seq of `messages[i]`; a missing seq counts as 0.
 */
export function indexChunks(
  messages: Message[],
  seqs: number[],
  tools: readonly HarnessTool[],
  count: TokenCounter,
): Chunk[] {
  const lookup = { byName: toolsByName(tools), results: resultsById(messages) };
  const chunks: Chunk[] = [];
  for (let index = 0; index < messages.length; index += 1) {
    const message = messages[index];
    if (!message) continue;
    const seq = seqs[index] ?? 0;
    chunks.push(...messageChunks(message, index, seq, lookup, count));
  }
  return chunks;
}

type ToolResultText = { content: string; isError: boolean };

/** Tools by name; the first tool with a name wins. */
function toolsByName(tools: readonly HarnessTool[]): Map<string, HarnessTool> {
  const byName = new Map<string, HarnessTool>();
  for (const tool of tools) {
    if (!byName.has(tool.def.name)) byName.set(tool.def.name, tool);
  }
  return byName;
}

/** Tool results by tool_use id; the first result for an id wins. */
function resultsById(
  messages: readonly Message[],
): Map<string, ToolResultText> {
  const results = new Map<string, ToolResultText>();
  for (const message of messages) {
    for (const block of message.content) {
      if (block.type !== "tool_result" || results.has(block.toolUseId)) {
        continue;
      }
      results.set(block.toolUseId, {
        content: block.content,
        isError: block.isError,
      });
    }
  }
  return results;
}

/**
 * The chunks one message contributes: none for a results-only message,
 * else its prose chunk (when it has prose or no tool calls) followed by one
 * chunk per tool_use.
 */
function messageChunks(
  message: Message,
  index: number,
  seq: number,
  lookup: {
    byName: ReadonlyMap<string, HarnessTool>;
    results: ReadonlyMap<string, ToolResultText>;
  },
  count: TokenCounter,
): Chunk[] {
  const onlyResults =
    message.content.length > 0 &&
    message.content.every((block) => block.type === "tool_result");
  if (onlyResults) return [];
  const toolUses = message.content.filter(
    (block): block is ToolUseBlock => block.type === "tool_use",
  );
  const prose = proseText(message);
  const chunks: Chunk[] = [];
  if (prose.length > 0 || toolUses.length === 0) {
    chunks.push({
      id: `seq:${seq}`,
      kind: message.role,
      messageIndex: index,
      tokens: count(prose),
      accesses: [],
    });
  }
  for (const block of toolUses) {
    const result = lookup.results.get(block.id);
    const tool = lookup.byName.get(block.name);
    chunks.push({
      id: `seq:${seq}#${block.id}`,
      kind: "tool",
      messageIndex: index,
      toolUseId: block.id,
      toolName: block.name,
      input: block.input,
      isError: result?.isError ?? false,
      tokens: count(inputText(block)) + count(result?.content ?? ""),
      accesses: tool ? tool.accesses(block.input) : [],
    });
  }
  return chunks;
}

/**
 * True when the view may be cut after message `index`: never after a user
 * message or an assistant message with tool_use, and never right before a
 * user message that carries tool_result blocks.
 */
export function canSplitAfter(
  messages: readonly Message[],
  index: number,
): boolean {
  const message = messages[index];
  if (message === undefined) return false;
  if (message.role === "user") return false;
  if (message.content.some((block) => block.type === "tool_use")) return false;
  const next = messages[index + 1];
  if (
    next?.role === "user" &&
    next.content.some((block) => block.type === "tool_result")
  ) {
    return false;
  }
  return true;
}

function proseText(message: Message): string {
  let text = "";
  for (const block of message.content) {
    if (block.type === "text" || block.type === "thinking") text += block.text;
  }
  return text;
}

function inputText(block: ToolUseBlock): string {
  if (block.rawInput.length > 0) return block.rawInput;
  if (block.input === undefined) return "";
  return JSON.stringify(block.input) ?? "";
}
