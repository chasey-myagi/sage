/**
 * Keyless replay of a recorded session.
 *
 * Contract: `session.jsonl` is a JsonlLogStore file for session id `session`.
 * `responses.json` is `StreamChunk[][]`, one script per model turn. Replay
 * reads the user text, model, and adapter defaults back out of that log,
 * plays the script through the faux adapter, and returns the new requests,
 * log, and per-turn manifests. The prepared call uses `contextWindow` 8000
 * and `pricing` null; the log does not store those. `SAGE_RECORD=1` is the
 * record lane switch.
 * This module does not open a network connection. The live recorder is #16.
 *
 * Intentionally not in the Log: the script, the switch, and the manifest
 * list collected at runtime. The session log is the recorded fact.
 */

import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import type {
  Message,
  ProviderAdapter,
  Request,
  StreamChunk,
} from "@sage/provider/contract";
import { JsonlLogStore } from "../log/jsonl.js";
import { MemoryLogStore } from "../log/memory.js";
import type { StoredEntry } from "../log/types.js";
import { AgentSession } from "../session.js";
import type { HarnessTool } from "../tools.js";
import { passthroughView } from "../view/passthrough.js";
import type { Manifest, ViewProvider } from "../view/types.js";
import { fauxAdapter } from "./faux.js";

const RECORD_SWITCH = "SAGE_RECORD";

export function loadCase(dir: string): {
  log: StoredEntry[];
  responses: StreamChunk[][];
  expectedDir: string;
} {
  const recording = path.join(dir, "session.jsonl");
  if (!existsSync(recording)) {
    throw new Error(`missing recording: ${recording}`);
  }
  const store = new JsonlLogStore({ dir });
  const responsesPath = path.join(dir, "responses.json");
  const parsed: unknown = JSON.parse(readFileSync(responsesPath, "utf8"));
  if (!isChunkScript(parsed)) {
    throw new Error(`responses.json must be StreamChunk[][]: ${responsesPath}`);
  }
  return {
    log: [...store.all("session")],
    responses: parsed,
    expectedDir: path.join(dir, "expected"),
  };
}

export async function replaySession(
  dir: string,
  opts: {
    viewProvider?: ViewProvider;
    system: string;
    tools: HarnessTool[];
  },
): Promise<{ requests: Request[]; log: StoredEntry[]; manifests: Manifest[] }> {
  if (process.env[RECORD_SWITCH] === "1") {
    throw new Error(
      "SAGE_RECORD=1 selects the record lane. The on-disk format is session.jsonl plus responses.json. The live network recorder is #16.",
    );
  }
  const { log, responses } = loadCase(dir);
  const adapter = fauxAdapter(responses, {
    api: recordedApi(log),
    prepared: {
      contextWindow: 8000,
      pricing: null,
      adapterDefaults: recordedDefaults(log),
    },
  });
  const store = new MemoryLogStore();
  const manifests: Manifest[] = [];
  const inner = opts.viewProvider ?? passthroughView;
  const session = new AgentSession({
    store,
    sessionId: "replay",
    adapter,
    model: recordedModel(log),
    system: opts.system,
    tools: opts.tools,
    viewProvider: {
      async view(ctx) {
        const compiled = await inner.view(ctx);
        manifests.push(structuredClone(compiled.manifest));
        return compiled;
      },
    },
  });
  await session.run(recordedUserText(log));
  return {
    requests: adapter.requests,
    log: [...store.all("replay")],
    manifests,
  };
}

function isChunkScript(value: unknown): value is StreamChunk[][] {
  return Array.isArray(value) && value.every((turn) => Array.isArray(turn));
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function recordedModel(log: readonly StoredEntry[]): {
  provider: string;
  model: string;
} {
  for (const entry of log) {
    if (entry.kind !== "request_header" || !isRecord(entry.payload)) continue;
    const { provider, model } = entry.payload;
    if (typeof provider === "string" && typeof model === "string") {
      return { provider, model };
    }
  }
  throw new Error("recording has no request_header model");
}

function recordedDefaults(
  log: readonly StoredEntry[],
): Record<string, unknown> {
  for (const entry of log) {
    if (entry.kind !== "request_header" || !isRecord(entry.payload)) continue;
    const defaults = entry.payload.adapterDefaults;
    if (isRecord(defaults)) return structuredClone(defaults);
  }
  throw new Error("recording has no adapter defaults");
}

function recordedApi(log: readonly StoredEntry[]): ProviderAdapter["api"] {
  for (const entry of log) {
    const api = entry.details?.api;
    if (api === "anthropic-messages" || api === "openai-chat") return api;
  }
  return "anthropic-messages";
}

function recordedUserText(log: readonly StoredEntry[]): string {
  for (const entry of log) {
    if (entry.kind !== "message" || !isUserText(entry.payload)) continue;
    const texts: string[] = [];
    for (const block of entry.payload.content) {
      if (block.type === "text") texts.push(block.text);
    }
    return texts.join("");
  }
  throw new Error("recording has no user message");
}

function isUserText(payload: unknown): payload is Message {
  if (!isRecord(payload) || payload.role !== "user") return false;
  return Array.isArray(payload.content);
}
