/**
 * Rebuild the model-facing head and messages for one turn from the Log.
 *
 * Contract: `view_epoch.details.prefixMessage` is the frozen prefix message.
 * Message entries before that turn's request_header are the history, projected
 * with the same repair as passthrough. Tail text comes from
 * `manifest.details.claimContent`, walking backward to the manifest that
 * stored this content hash. System and tool text come from the latest
 * request_header that still carries them for the same hash. A manifest chunk
 * at tier none with a tool chunk id rebuilds that tool_result as the same
 * stub, using `originalTokens` and the logged tool_use. A summary chunk
 * rebuilds that stub, a newline, and
 * `summary (model notes, not evidence): ` plus the logged summary string.
 * An epoch head that is already one of the projected history messages is not
 * inserted again. The compiler leaves that message in place when it is a
 * logged history message rather than a claim.
 *
 * Intentionally not in the Log: this function. It only reads entries the
 * kernel already wrote.
 */

import type { StoredEntry, ViewContext } from "@sage/core";
import { isRecord, passthroughView, prefixMessageFor } from "@sage/core";
import type {
  Head,
  Message,
  ToolDef,
  ToolUseBlock,
} from "@sage/provider/contract";
import { renderSummary, replaceToolResults } from "./apply.js";
import { type Chunk, toolUseIdOfChunk } from "./chunks.js";
import { renderStub } from "./stub.js";

type LoggedClaim = {
  id: string;
  tier: string;
  placement?: string;
  priority?: number;
  contentHash?: string;
};

/**
 * Rebuilds the head and messages the model saw on `turn`. Throws when the
 * path has no manifest or request_header for that turn, when a tail claim's
 * content is missing from every manifest, or when the projection is not
 * synchronous.
 */
export function rebuildFromLog(
  path: readonly StoredEntry[],
  turn: number,
): { head: Head; messages: Message[] } {
  const manifestIndex = findManifest(path, turn);
  const headerIndex = findHeader(path, manifestIndex);
  const head = resolveHead(path, headerIndex);
  const prefix = path.slice(0, headerIndex);
  const projected = passthroughView.view(rebuildContext(prefix, turn, head));
  if (projected instanceof Promise) {
    throw new Error("passthrough projection must be synchronous");
  }
  const manifest = path[manifestIndex];
  if (!manifest) throw new Error(`no manifest for turn ${turn}`);
  const history = replaceToolResults(
    projected.messages,
    stubsFor(projected.messages, manifest),
  );
  const epoch = epochOf(manifest);
  const prefixMessage = prefixMessageFor(prefix, epoch);
  const messages = [...history];
  if (
    prefixMessage &&
    !history.some((message) => sameMessage(message, prefixMessage))
  ) {
    messages.unshift(prefixMessage);
  }
  appendTail(messages, tailBlocks(path, manifestIndex));
  return { head, messages };
}

function rebuildContext(
  path: readonly StoredEntry[],
  turn: number,
  head: Head,
): ViewContext {
  const model = { provider: "rebuild", model: "rebuild" };
  return {
    sessionId: "rebuild",
    turn,
    path,
    model,
    prepared: {
      model,
      contextWindow: 0,
      pricing: null,
      adapterDefaults: {},
    },
    head,
    tools: [],
    depth: 0,
  };
}

function findManifest(path: readonly StoredEntry[], turn: number): number {
  let found = -1;
  for (let index = 0; index < path.length; index += 1) {
    const entry = path[index];
    if (entry?.kind !== "manifest" || !isRecord(entry.payload)) continue;
    if (entry.payload.turn === turn) found = index;
  }
  if (found < 0) throw new Error(`no manifest for turn ${turn}`);
  return found;
}

function findHeader(
  path: readonly StoredEntry[],
  manifestIndex: number,
): number {
  for (let index = manifestIndex - 1; index >= 0; index -= 1) {
    if (path[index]?.kind === "request_header") return index;
  }
  throw new Error("manifest has no request_header");
}

function resolveHead(path: readonly StoredEntry[], headerIndex: number): Head {
  const header = headerPayload(path[headerIndex]);
  let system = header.system;
  let tools = header.tools;
  for (
    let index = headerIndex - 1;
    index >= 0 && (system === undefined || tools === undefined);
    index -= 1
  ) {
    if (path[index]?.kind !== "request_header") continue;
    const previous = headerPayload(path[index]);
    if (
      system === undefined &&
      previous.system !== undefined &&
      previous.systemPromptHash === header.systemPromptHash
    ) {
      system = previous.system;
    }
    if (
      tools === undefined &&
      previous.tools !== undefined &&
      previous.toolsHash === header.toolsHash
    ) {
      tools = previous.tools;
    }
  }
  return { system: system ?? "", tools: tools ?? [] };
}

function headerPayload(entry: StoredEntry | undefined): {
  systemPromptHash?: string;
  toolsHash?: string;
  system?: string;
  tools?: ToolDef[];
} {
  if (!entry || !isRecord(entry.payload)) return {};
  const payload = entry.payload;
  return {
    systemPromptHash:
      typeof payload.systemPromptHash === "string"
        ? payload.systemPromptHash
        : undefined,
    toolsHash:
      typeof payload.toolsHash === "string" ? payload.toolsHash : undefined,
    system: typeof payload.system === "string" ? payload.system : undefined,
    tools: Array.isArray(payload.tools)
      ? (payload.tools as ToolDef[])
      : undefined,
  };
}

function sameMessage(left: Message, right: Message): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

function epochOf(entry: StoredEntry): number {
  if (!isRecord(entry.payload)) return 0;
  return typeof entry.payload.epoch === "number" ? entry.payload.epoch : 0;
}

function tailBlocks(
  path: readonly StoredEntry[],
  manifestIndex: number,
): { type: "text"; text: string }[] {
  const manifest = path[manifestIndex];
  const claims = loggedClaims(manifest).filter(
    (claim) =>
      (claim.placement === "tail" || claim.placement === "tail-if-changed") &&
      claim.tier !== "none",
  );
  claims.sort(
    (left, right) =>
      (right.priority ?? 0) - (left.priority ?? 0) ||
      left.id.localeCompare(right.id),
  );
  return claims.map((claim) => ({
    type: "text" as const,
    text: textFor(path, manifestIndex, claim),
  }));
}

function textFor(
  path: readonly StoredEntry[],
  manifestIndex: number,
  claim: LoggedClaim,
): string {
  for (let index = manifestIndex; index >= 0; index -= 1) {
    const entry = path[index];
    if (entry?.kind !== "manifest") continue;
    const match = loggedClaims(entry).find(
      (item) => item.id === claim.id && item.contentHash === claim.contentHash,
    );
    if (!match) continue;
    const stored = entry.details?.claimContent;
    if (!isRecord(stored)) continue;
    const text = stored[claim.id];
    if (typeof text === "string") return text;
  }
  throw new Error(`missing claim content for ${claim.id}`);
}

function loggedClaims(entry: StoredEntry | undefined): LoggedClaim[] {
  if (
    !entry ||
    !isRecord(entry.payload) ||
    !Array.isArray(entry.payload.claims)
  ) {
    return [];
  }
  const claims: LoggedClaim[] = [];
  for (const value of entry.payload.claims) {
    if (!isRecord(value) || typeof value.id !== "string") continue;
    if (typeof value.tier !== "string") continue;
    claims.push({
      id: value.id,
      tier: value.tier,
      placement:
        typeof value.placement === "string" ? value.placement : undefined,
      priority: typeof value.priority === "number" ? value.priority : undefined,
      contentHash:
        typeof value.contentHash === "string" ? value.contentHash : undefined,
    });
  }
  return claims;
}

function appendTail(
  messages: Message[],
  blocks: { type: "text"; text: string }[],
): void {
  if (blocks.length === 0) return;
  const last = messages.at(-1);
  if (last?.role === "user") {
    last.content.push(...blocks);
    return;
  }
  messages.push({ role: "user", content: blocks });
}

function stubsFor(
  messages: readonly Message[],
  entry: StoredEntry | undefined,
): Map<string, string> {
  const stubs = new Map<string, string>();
  for (const chunk of loggedChunks(entry)) {
    if (chunk.tier !== "none" && chunk.tier !== "summary") continue;
    const toolUseId = toolUseIdOfChunk(chunk.id);
    if (!toolUseId) continue;
    const use = findToolUse(messages, toolUseId);
    if (!use) continue;
    const tokens =
      typeof chunk.originalTokens === "number" ? chunk.originalTokens : 0;
    const rendered: Chunk = {
      id: chunk.id,
      kind: "tool",
      messageIndex: 0,
      toolUseId,
      toolName: use.name,
      input: use.input,
      tokens,
      accesses: [],
    };
    if (chunk.tier === "summary") {
      stubs.set(toolUseId, renderSummary(rendered, chunk.summary ?? ""));
      continue;
    }
    stubs.set(toolUseId, renderStub(rendered));
  }
  return stubs;
}

function loggedChunks(entry: StoredEntry | undefined): {
  id: string;
  tier: string;
  originalTokens?: number;
  summary?: string;
}[] {
  if (
    !entry ||
    !isRecord(entry.payload) ||
    !Array.isArray(entry.payload.chunks)
  ) {
    return [];
  }
  const chunks: {
    id: string;
    tier: string;
    originalTokens?: number;
    summary?: string;
  }[] = [];
  for (const value of entry.payload.chunks) {
    if (!isRecord(value) || typeof value.id !== "string") continue;
    if (typeof value.tier !== "string") continue;
    chunks.push({
      id: value.id,
      tier: value.tier,
      originalTokens:
        typeof value.originalTokens === "number"
          ? value.originalTokens
          : undefined,
      summary: typeof value.summary === "string" ? value.summary : undefined,
    });
  }
  return chunks;
}

function findToolUse(
  messages: readonly Message[],
  toolUseId: string,
): ToolUseBlock | undefined {
  for (const message of messages) {
    for (const block of message.content) {
      if (block.type === "tool_use" && block.id === toolUseId) return block;
    }
  }
  return undefined;
}
