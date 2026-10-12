/**
 * Keyless replay of a recorded session.
 *
 * Guarantees: `session.jsonl` is a JsonlLogStore file for session id
 * `session`. `responses.json` is `StreamChunk[][]`, one script per model
 * turn. Replay reads the user text, model, adapter defaults, context window,
 * and pricing back out of that log (the first request_header), plays the
 * script through the faux adapter, and returns the new requests, log,
 * per-turn compiled views, and their manifests. This module never opens a
 * network connection.
 *
 * Intentionally not in the Log: the script and the view and manifest lists
 * collected at runtime. The session log is the recorded fact.
 */

import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import {
  isRecord,
  type Message,
  type PreparedCall,
  type ProviderAdapter,
  type Request,
  type StreamChunk,
} from "@sage/provider/contract";
import { JsonlLogStore } from "../log/jsonl.js";
import type { RequestHeaderPayload } from "../log/kinds.js";
import { MemoryLogStore } from "../log/memory.js";
import { requestHeaderOf } from "../log/path.js";
import type { StoredEntry } from "../log/types.js";
import { AgentSession } from "../session.js";
import type { HarnessTool } from "../tools.js";
import { passthroughView } from "../view/passthrough.js";
import type { CompiledView, Manifest, ViewProvider } from "../view/types.js";
import { fauxAdapter } from "./faux.js";
import { recordViews } from "./views.js";

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
 * Returns the requests sent, the resulting log, every compiled view in turn
 * order, and the manifests derived from those views. Throws when the
 * recording lacks a user message or a request_header.
 */
export async function replaySession(
  dir: string,
  opts: {
    viewProvider?: ViewProvider;
    system: string;
    tools: HarnessTool[];
  },
): Promise<{
  requests: Request[];
  log: StoredEntry[];
  views: CompiledView[];
  manifests: Manifest[];
}> {
  const { log, responses } = loadCase(dir);
  const adapter = fauxAdapter(responses, {
    api: recordedApi(log),
    prepared: recordedPrepared(log),
  });
  const store = new MemoryLogStore();
  const recorder = recordViews(opts.viewProvider ?? passthroughView);
  const model = recordedModel(log);
  const session = new AgentSession({
    store,
    sessionId: "replay",
    adapters: { [model.provider]: adapter },
    model,
    system: opts.system,
    tools: opts.tools,
    viewProvider: recorder,
  });
  await session.run(recordedUserText(log));
  return {
    requests: adapter.requests,
    log: [...store.all("replay")],
    views: recorder.views,
    manifests: recorder.views.map((view) => view.manifest),
  };
}

function isChunkScript(value: unknown): value is StreamChunk[][] {
  return Array.isArray(value) && value.every((turn) => Array.isArray(turn));
}

function firstHeader(log: readonly StoredEntry[]): RequestHeaderPayload {
  for (const entry of log) {
    const header = requestHeaderOf(entry);
    if (header) return header;
  }
  throw new Error("recording has no request_header");
}

function recordedModel(log: readonly StoredEntry[]): {
  provider: string;
  model: string;
} {
  const { provider, model } = firstHeader(log);
  return { provider, model };
}

function recordedPrepared(
  log: readonly StoredEntry[],
): Omit<PreparedCall, "model"> {
  const header = structuredClone(firstHeader(log));
  return {
    contextWindow: header.contextWindow,
    pricing: header.pricing,
    adapterDefaults: header.adapterDefaults,
  };
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
