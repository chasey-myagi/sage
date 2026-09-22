import path from "node:path";
import { fileURLToPath } from "node:url";
import type { HarnessTool } from "../../src/tools.ts";

const replayRoot = path.dirname(fileURLToPath(import.meta.url));

export const REPLAY_MODEL = { provider: "anthropic", model: "claude-test" };

export const REPLAY_DEFAULTS = { maxTokens: 64, temperature: 0 };

export type ReplayCase = {
  name: string;
  /** Prompt recorded into session.jsonl. Replay reads the text back from there. */
  user: string;
  system: string;
  tools: HarnessTool[];
};

export function echoTool(): HarnessTool {
  return {
    def: {
      name: "echo",
      description: "Echo text",
      inputSchema: {
        type: "object",
        properties: { text: { type: "string" } },
      },
    },
    isReadOnly: true,
    isDestructive: false,
    isOpenWorld: false,
    accesses() {
      return [];
    },
    async execute(input: unknown) {
      const text =
        typeof input === "object" &&
        input !== null &&
        "text" in input &&
        typeof input.text === "string"
          ? input.text
          : "";
      return { content: text, isError: false };
    },
  };
}

export const REPLAY_CASES: ReplayCase[] = [
  {
    name: "text-only",
    user: "hello",
    system: "You are sage.",
    tools: [],
  },
  {
    name: "tool-roundtrip",
    user: "ping",
    system: "You are sage. Use tools when asked.",
    tools: [echoTool()],
  },
];

export function caseDir(name: string): string {
  return path.join(replayRoot, name);
}
