/**
 * Writes one model call's request entries: `view_epoch` when due,
 * `request_header`, then `manifest`. The one implementation of the
 * request_header content addressing; `AgentSession` and offline replay both
 * append through it.
 *
 * Guarantees: entries are appended in that order and nothing else is
 * appended. `system` and `tools` full text is stored only when its hash is
 * new relative to the previous request_header on the path.
 *
 * Intentionally not in the Log: the CompiledView itself. Its messages are
 * already message entries; only `messages[0]` is copied, as the
 * view_epoch's `prefixMessage`.
 */

import type { ModelRef, PreparedCall } from "@sage/provider/contract";
import type { RequestHeaderPayload } from "../log/kinds.js";
import { lastRequestHeader, latestViewEpoch } from "../log/path.js";
import type { LogStore } from "../log/store.js";
import { systemHash, toolsHash } from "./assert.js";
import type { CompiledView } from "./types.js";

/** What one call's request entries are built from. */
type RequestEntriesInput = {
  view: CompiledView;
  /** The model the request goes to: the header's `provider` and `model`. */
  model: ModelRef;
  /** Effective sampling of the call, copied onto the header. */
  sampling: Record<string, unknown>;
  prepared: PreparedCall;
};

/**
 * Appends this call's `view_epoch` (when due), `request_header`, and
 * `manifest` to `sessionId`.
 *
 * `view_epoch {epoch, reason}` is appended when `view.epochStart` is set,
 * or when `view.epoch` is greater than the latest view_epoch on the path
 * (0 when there is none). `reason` is `epochStart.reason`, else
 * `epoch-advanced`; `details.prefixMessage` is the view's first message.
 *
 * The request_header records `model`, a copy of `sampling`, and
 * `prepared`'s adapter defaults, context window, and pricing.
 *
 * The manifest payload is `view.manifest`; its details are `view.details`.
 * Throws `InvalidEntryError` from the store when an entry fails validation
 * (for example a non-positive context window); entries appended before the
 * failing one stay.
 */
export function appendRequestEntries(
  store: LogStore,
  sessionId: string,
  input: RequestEntriesInput,
): void {
  appendViewEpoch(store, sessionId, input.view);
  appendHeader(store, sessionId, input);
  store.append(sessionId, {
    kind: "manifest",
    payload: input.view.manifest,
    ...(input.view.details ? { details: input.view.details } : {}),
  });
}

function appendViewEpoch(
  store: LogStore,
  sessionId: string,
  view: CompiledView,
): void {
  const latest = latestViewEpoch(store.path(sessionId));
  if (!view.epochStart && view.epoch <= latest) return;
  const firstMessage = view.messages[0];
  store.append(sessionId, {
    kind: "view_epoch",
    payload: {
      epoch: view.epoch,
      reason: view.epochStart?.reason ?? "epoch-advanced",
    },
    ...(firstMessage ? { details: { prefixMessage: firstMessage } } : {}),
  });
}

function appendHeader(
  store: LogStore,
  sessionId: string,
  input: RequestEntriesInput,
): void {
  const { view, prepared } = input;
  const prev = lastRequestHeader(store.path(sessionId));
  const systemPromptHash = systemHash(view.head.system);
  const toolsDigest = toolsHash(view.head.tools);
  const payload: RequestHeaderPayload = {
    provider: input.model.provider,
    model: input.model.model,
    systemPromptHash,
    toolsHash: toolsDigest,
    sampling: { ...input.sampling },
    adapterDefaults: { ...prepared.adapterDefaults },
    contextWindow: prepared.contextWindow,
    pricing: prepared.pricing ? structuredClone(prepared.pricing) : null,
  };
  if (!prev || prev.systemPromptHash !== systemPromptHash) {
    payload.system = view.head.system;
  }
  if (!prev || prev.toolsHash !== toolsDigest) {
    payload.tools = structuredClone(view.head.tools);
  }
  store.append(sessionId, { kind: "request_header", payload });
}
