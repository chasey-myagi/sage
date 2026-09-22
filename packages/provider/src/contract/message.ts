/**
 * Provider-neutral request vocabulary.
 *
 * Deliberately not a Log entry: a request is compiled view plus sampling.
 * The Log records it later as a request_header entry (#4). API keys, fetch
 * signals, and wire bytes never belong in that entry.
 */

export type Role = "user" | "assistant";

export type TextBlock = { type: "text"; text: string };

export type ThinkingBlock = {
  type: "thinking";
  text: string;
  signature?: string;
  redacted?: boolean;
};

export type ToolUseBlock = {
  type: "tool_use";
  id: string;
  name: string;
  input: unknown;
  rawInput: string;
};

export type ToolResultBlock = {
  type: "tool_result";
  toolUseId: string;
  content: string;
  isError: boolean;
};

export type ContentBlock =
  | TextBlock
  | ThinkingBlock
  | ToolUseBlock
  | ToolResultBlock;

export type Message = { role: Role; content: ContentBlock[] };

export type ToolDef = {
  name: string;
  description: string;
  inputSchema: Record<string, unknown>;
};

export type Head = { system: string; tools: ToolDef[] };

export type CacheBreakpoint = "head" | { afterMessage: number };

export type ModelRef = { provider: string; model: string };

export type Sampling = { maxTokens?: number; temperature?: number };

export type Request = {
  model: ModelRef;
  head: Head;
  messages: Message[];
  cacheBreakpoints: CacheBreakpoint[];
  sampling: Sampling;
};
