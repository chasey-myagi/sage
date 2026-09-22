/**
 * Rebuild the model-facing head and messages for one turn from the Log.
 *
 * Contract: `view_epoch.details.epochHead` is the frozen prefix message.
 * Message entries before that turn's request_header are the history, projected
 * with the same repair as passthrough. Tail text comes from
 * `manifest.details.claimContent`, walking backward to the manifest that
 * stored this content hash. System and tool text come from the latest
 * request_header that still carries them for the same hash. A manifest chunk
 * at tier none whose id contains `#` rebuilds that tool_result as the same
 * stub, using `originalTokens` and the logged tool_use.
 *
 * Intentionally not in the Log: this function. It only reads entries the
 * kernel already wrote.
 */

import type { StoredEntry, ViewContext } from "@sage/core";
import { passthroughView } from "@sage/core";
import type {
  Head,
  Message,
  ToolDef,
  ToolUseBlock,
} from "@sage/provider/contract";
import { replaceToolResults } from "./apply.js";
import type { Chunk } from "./chunks.js";
import { renderStub } from "./stub.js";

type LoggedClaim = {
  id: string;
  tier: string;
  placement?: string;
  priority?: number;
  contentHash?: string;
};

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
  const epochHead = epochHeadFor(prefix, epoch);
  const messages = [...history];
  if (epochHead) messages.unshift(structuredClone(epochHead));
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

function epochOf(entry: StoredEntry): number {
  if (!isRecord(entry.payload)) return 0;
  return typeof entry.payload.epoch === "number" ? entry.payload.epoch : 0;
}

function epochHeadFor(
  path: readonly StoredEntry[],
  epoch: number,
): Message | undefined {
  let found: Message | undefined;
  for (const entry of path) {
    if (entry.kind !== "view_epoch" || !isRecord(entry.payload)) continue;
    if (entry.payload.epoch !== epoch) continue;
    const head = entry.details?.epochHead;
    if (!isMessage(head)) continue;
    found = head;
  }
  return found;
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
    if (chunk.tier !== "none") continue;
    const mark = chunk.id.indexOf("#");
    if (mark < 0) continue;
    const toolUseId = chunk.id.slice(mark + 1);
    if (toolUseId.length === 0) continue;
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
    stubs.set(toolUseId, renderStub(rendered));
  }
  return stubs;
}

function loggedChunks(entry: StoredEntry | undefined): {
  id: string;
  tier: string;
  originalTokens?: number;
}[] {
  if (
    !entry ||
    !isRecord(entry.payload) ||
    !Array.isArray(entry.payload.chunks)
  ) {
    return [];
  }
  const chunks: { id: string; tier: string; originalTokens?: number }[] = [];
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

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isMessage(value: unknown): value is Message {
  if (!isRecord(value)) return false;
  return (
    (value.role === "user" || value.role === "assistant") &&
    Array.isArray(value.content)
  );
}
