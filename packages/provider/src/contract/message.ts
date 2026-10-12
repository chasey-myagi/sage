/**
 * Provider-neutral request types: messages, content blocks, tools, and the
 * request an adapter encodes.
 *
 * Guarantees: these types are this repository's own; no SDK type leaks
 * through them.
 *
 * Intentionally not in the Log: a request as such. It is a compiled view
 * plus sampling; the session records its messages as message entries and
 * its envelope as a request_header entry. API keys, fetch signals, and wire
 * bytes never belong in either.
 */

/** Who wrote a message. Tool results travel in user messages. */
export type Role = "user" | "assistant";

/** Plain text. */
export type TextBlock = { type: "text"; text: string };

/**
 * Model reasoning. `signature` is the provider's opaque check value;
 * `redacted` marks reasoning the provider returned only in encrypted form.
 */
export type ThinkingBlock = {
  type: "thinking";
  text: string;
  signature?: string;
  redacted?: boolean;
};

/**
 * A tool call. `rawInput` is the JSON text as streamed; `input` is its parse
 * (`{}` for blank text), or null when it did not parse. Adapters replay a
 * call whose `input` is not an object through `replayToolInput`.
 */
export type ToolUseBlock = {
  type: "tool_use";
  id: string;
  name: string;
  input: unknown;
  rawInput: string;
};

/** The answer to the tool_use whose id is `toolUseId`. */
export type ToolResultBlock = {
  type: "tool_result";
  toolUseId: string;
  content: string;
  isError: boolean;
};

/** Any block a message can hold. */
export type ContentBlock =
  | TextBlock
  | ThinkingBlock
  | ToolUseBlock
  | ToolResultBlock;

/** One conversation message. */
export type Message = { role: Role; content: ContentBlock[] };

/** A tool schema as the model sees it. `inputSchema` is JSON Schema. */
export type ToolDef = {
  name: string;
  description: string;
  inputSchema: Record<string, unknown>;
};

/**
 * The head: system prompt plus tool schemas, the part of every request that
 * precedes the messages. "Head" means only this; the persisted fields that
 * once reused the word are `calibration.overheadTokens` and
 * `details.prefixMessage` (docs/notes/implemented/2026-09-24-head-naming.md).
 */
export type Head = { system: string; tools: ToolDef[] };

/**
 * Where a cache marker goes: after the head, or after message index
 * `afterMessage`. Adapters whose wire format has no cache field ignore it.
 */
export type CacheBreakpoint = "head" | { afterMessage: number };

/** A model by provider name and model id. */
export type ModelRef = { provider: string; model: string };

/** Sampling overrides; an absent field falls back to adapter defaults. */
export type Sampling = { maxTokens?: number; temperature?: number };

/**
 * `auto` or `none`. Adapters map it onto the wire and omit the field when
 * the request leaves it unset.
 */
export type ToolChoice = "auto" | "none";

/** Everything an adapter needs to encode one call. */
export type Request = {
  model: ModelRef;
  head: Head;
  messages: Message[];
  cacheBreakpoints: CacheBreakpoint[];
  sampling: Sampling;
  toolChoice?: ToolChoice;
};
