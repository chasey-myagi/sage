/**
 * Keyless replay of a recorded session.
 *
 * Guarantees: `session.jsonl` is a JsonlLogStore file for session id
 * `session`. `responses.json` is `StreamChunk[][]`, one script per model
 * turn. Replay reads the user text, model, and adapter defaults back out of
 * that log, plays the script through the faux adapter, and returns the new
 * requests, log, and per-turn manifests. The prepared call uses
 * `FAUX_CONTEXT_WINDOW` and `pricing` null; the log does not store those.
 * `SAGE_RECORD=1` selects the record lane, which has no live network
 * recorder yet, so replay refuses it. This module never opens a network
 * connection.
 *
 * Intentionally not in the Log: the script, the switch, and the manifest
 * list collected at runtime. The session log is the recorded fact.
 */

import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import {
  isRecord,
  type Message,
  type ProviderAdapter,
  type Request,
  type StreamChunk,
} from "@sage/provider/contract";
import { JsonlLogStore } from "../log/jsonl.js";
import { MemoryLogStore } from "../log/memory.js";
import type { StoredEntry } from "../log/types.js";
import { AgentSession } from "../session.js";
import type { HarnessTool } from "../tools.js";
import { passthroughView } from "../view/passthrough.js";
import type { Manifest, ViewProvider } from "../view/types.js";
import { FAUX_CONTEXT_WINDOW, fauxAdapter } from "./faux.js";

const RECORD_SWITCH = "SAGE_RECORD";

/**
 * Reads a recorded case directory. Throws when `session.jsonl` is missing or
 * `responses.json` is not `StreamChunk[][]`.
 */
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

/**
 * Replays a recorded case keylessly through a fresh in-memory session.
 * Throws when `SAGE_RECORD=1`, and when the recording lacks a user message
 * or a request_header model and adapter defaults.
 */
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
      "SAGE_RECORD=1 selects the record lane, which replaySession does not implement: it only replays session.jsonl plus responses.json from disk and never calls the network. Unset SAGE_RECORD to replay.",
    );
  }
  const { log, responses } = loadCase(dir);
  const adapter = fauxAdapter(responses, {
    api: recordedApi(log),
    prepared: {
      contextWindow: FAUX_CONTEXT_WINDOW,
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
