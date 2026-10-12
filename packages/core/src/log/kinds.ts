/**
 * Kernel entry kinds, registered when this module is imported.
 *
 * Guarantees: each validator checks shape only and rejects anything else.
 * `leaf` is kernel semantics (it moves the pointer); readers interpret the
 * other kinds. The kernel never interprets manifest reports,
 * `terminal.health`, or the `subtask` payload beyond their shape. Only `@sage/provider/contract` is
 * imported, never the adapters.
 *
 * Per-kind rules are on each payload type below.
 *
 * Intentionally not in the Log: the registry.
 */

import {
  isFiniteNumber,
  isModelRef,
  isPricing,
  isRecord,
  type Message,
  type ModelRef,
  type Pricing,
} from "@sage/provider/contract";
import { registerEntryKind } from "./registry.js";

/**
 * The request envelope for one model call. `system` and `tools` carry full
 * text only when their hash differs from the previous request_header on
 * the path, or when there is none; `appendRequestEntries` decides, the
 * validator only checks the shape.
 */
export type RequestHeaderPayload = {
  provider: string;
  model: string;
  systemPromptHash: string;
  toolsHash: string;
  sampling: Record<string, unknown>;
  adapterDefaults: Record<string, unknown>;
  /**
   * `PreparedCall.contextWindow` of this call, in tokens: a positive
   * finite number. Replay reads it instead of the current catalog.
   */
  contextWindow: number;
  /**
   * `PreparedCall.pricing` of this call, or null when the adapter had no
   * catalog price. Replay reads it instead of the current catalog.
   */
  pricing: Pricing | null;
  system?: string;
  tools?: unknown;
};

/**
 * A new view epoch opened. The entry's `details.prefixMessage` is the
 * first message of the view that opened it: the message in the frozen
 * prefix that carries the epoch's claims.
 */
type ViewEpochPayload = {
  epoch: number;
  reason: string;
};

/** A model switch. History is not rewritten. */
type ModelChangePayload = {
  from: ModelRef;
  to: ModelRef;
  reason: string;
};

/**
 * Stored manifest. `budget` is a finite number or an object with finite
 * `total`, `used`, and `overrun` (and finite `frozenTokens` and
 * `staleOmittable` when present). `epochDecision`, `calibration`, and
 * `cache` are optional objects; `epochDecision.remainingTurns` must be
 * finite, and `calibration`'s `measured`, `estimated`, `ratio`, and
 * `overheadTokens` are finite when present. `shadow` needs a string
 * `scorer`, a finite `calls`, and a `decisions` array of `{id, tier}`
 * strings with optional string `reason`; optional `fallback` is a string.
 * `entryScored` needs finite `calls`, `none`, `summary`, and `full`, and an optional
 * finite `summaryRejected`. A `usage` on `shadow` or `entryScored` is an
 * object whose four token fields are finite when present. A `raw` on
 * `shadow` or `scorer` is `{text, finish}` strings or absent. A
 * `failedRequests` on `shadow`, `scorer`, or `entryScored` is finite when
 * present. Other fields
 * are unchecked. The kernel reads none of this.
 */
type ManifestPayload = {
  epoch: number;
  turn: number;
  budget?:
    | number
    | {
        total: number;
        used: number;
        overrun: number;
        frozenTokens?: number;
        staleOmittable?: number;
      };
  claims: unknown[];
  chunks: unknown[];
  anomalies: unknown[];
  epochDecision?: Record<string, unknown>;
  calibration?: Record<string, unknown>;
  cache?: Record<string, unknown>;
  shadow?: unknown;
  entryScored?: unknown;
};

/** Payload of `undo` and `leaf`: the id of an earlier entry. */
type TargetPayload = {
  target: string;
};

/**
 * One whole-turn resend after a transport error or a transient in-stream
 * error event. Persisted, not model-visible: the kernel never puts it on a
 * request.
 */
type TurnRetryPayload = {
  turn: number;
  attempt: number;
  reason: string;
};

/**
 * How a run ended. `usage` and `error` stay opaque. `health`, when present,
 * must be an object; the kernel does not read its fields.
 */
type TerminalPayload = {
  outcome: string;
  usage?: unknown;
  error?: unknown;
  /** Opaque run-health object. Counts only; the kernel does not read it. */
  health?: Record<string, unknown>;
};

/** `role` is a string and `content` is present; blocks are not checked. */
function isMessage(payload: unknown): payload is Message {
  return (
    isRecord(payload) &&
    typeof payload.role === "string" &&
    "content" in payload
  );
}

function isRequestHeader(payload: unknown): payload is RequestHeaderPayload {
  if (!isRecord(payload)) return false;
  if (typeof payload.provider !== "string") return false;
  if (typeof payload.model !== "string") return false;
  if (typeof payload.systemPromptHash !== "string") return false;
  if (typeof payload.toolsHash !== "string") return false;
  if (!isRecord(payload.sampling)) return false;
  if (!isRecord(payload.adapterDefaults)) return false;
  if (!isPositiveFinite(payload.contextWindow)) return false;
  if (payload.pricing !== null && !isPricing(payload.pricing)) return false;
  if (payload.system !== undefined && typeof payload.system !== "string") {
    return false;
  }
  return true;
}

function isPositiveFinite(value: unknown): value is number {
  return isFiniteNumber(value) && value > 0;
}

function isViewEpoch(payload: unknown): payload is ViewEpochPayload {
  return (
    isRecord(payload) &&
    isFiniteNumber(payload.epoch) &&
    typeof payload.reason === "string"
  );
}

function isModelChange(payload: unknown): payload is ModelChangePayload {
  return (
    isRecord(payload) &&
    isModelRef(payload.from) &&
    isModelRef(payload.to) &&
    typeof payload.reason === "string"
  );
}

function isManifest(payload: unknown): payload is ManifestPayload {
  if (!isRecord(payload)) return false;
  if (!isFiniteNumber(payload.epoch)) {
    return false;
  }
  if (!isFiniteNumber(payload.turn)) {
    return false;
  }
  if (payload.budget !== undefined && !isBudget(payload.budget)) return false;
  if (
    payload.epochDecision !== undefined &&
    !isEpochDecision(payload.epochDecision)
  ) {
    return false;
  }
  if (
    payload.calibration !== undefined &&
    !isCalibration(payload.calibration)
  ) {
    return false;
  }
  if (payload.cache !== undefined && !isRecord(payload.cache)) return false;
  if (payload.shadow !== undefined && !isShadow(payload.shadow)) return false;
  if (payload.scorer !== undefined && !isScorer(payload.scorer)) return false;
  if (
    payload.entryScored !== undefined &&
    !isEntryScored(payload.entryScored)
  ) {
    return false;
  }
  return (
    Array.isArray(payload.claims) &&
    Array.isArray(payload.chunks) &&
    Array.isArray(payload.anomalies)
  );
}

function isShadow(value: unknown): boolean {
  if (!isRecord(value)) return false;
  if (typeof value.scorer !== "string") return false;
  if (!Array.isArray(value.decisions)) return false;
  if (value.fallback !== undefined && typeof value.fallback !== "string") {
    return false;
  }
  if (!isFiniteNumber(value.calls)) return false;
  if (value.usage !== undefined && !isShadowUsage(value.usage)) return false;
  if (!rawFieldOk(value)) return false;
  if (!isOptionalFinite(value.failedRequests)) return false;
  return value.decisions.every(isShadowDecision);
}

function isScorer(value: unknown): boolean {
  if (!rawFieldOk(value)) return false;
  return !isRecord(value) || isOptionalFinite(value.failedRequests);
}

function rawFieldOk(value: unknown): boolean {
  if (!isRecord(value) || value.raw === undefined) return true;
  const raw = value.raw;
  return (
    isRecord(raw) &&
    typeof raw.text === "string" &&
    typeof raw.finish === "string"
  );
}

function isShadowDecision(value: unknown): boolean {
  if (!isRecord(value)) return false;
  if (typeof value.id !== "string") return false;
  if (typeof value.tier !== "string") return false;
  if (value.reason !== undefined && typeof value.reason !== "string") {
    return false;
  }
  return true;
}

function isEntryScored(value: unknown): boolean {
  if (!isRecord(value)) return false;
  if (!isFiniteNumber(value.calls)) return false;
  if (!isFiniteNumber(value.none)) return false;
  if (!isFiniteNumber(value.summary)) return false;
  if (!isFiniteNumber(value.full)) return false;
  if (!isOptionalFinite(value.summaryRejected)) return false;
  if (!isOptionalFinite(value.failedRequests)) return false;
  if (value.usage !== undefined && !isShadowUsage(value.usage)) return false;
  return true;
}

function isShadowUsage(value: unknown): boolean {
  if (!isRecord(value)) return false;
  return (
    isOptionalFinite(value.inputOther) &&
    isOptionalFinite(value.inputCacheRead) &&
    isOptionalFinite(value.inputCacheCreation) &&
    isOptionalFinite(value.output)
  );
}

function isEpochDecision(value: unknown): boolean {
  if (!isRecord(value)) return false;
  return isFiniteNumber(value.remainingTurns);
}

function isCalibration(value: unknown): boolean {
  if (!isRecord(value)) return false;
  return (
    isOptionalFinite(value.measured) &&
    isOptionalFinite(value.estimated) &&
    isOptionalFinite(value.ratio) &&
    isOptionalFinite(value.overheadTokens)
  );
}

function isBudget(value: unknown): value is
  | number
  | {
      total: number;
      used: number;
      overrun: number;
      frozenTokens?: number;
      staleOmittable?: number;
    } {
  if (typeof value === "number") return isFiniteNumber(value);
  if (!isRecord(value)) return false;
  if (!isFiniteNumber(value.total)) return false;
  if (!isFiniteNumber(value.used)) return false;
  if (!isFiniteNumber(value.overrun)) return false;
  if (!isOptionalFinite(value.frozenTokens)) return false;
  if (!isOptionalFinite(value.staleOmittable)) return false;
  return true;
}

function isOptionalFinite(value: unknown): boolean {
  return value === undefined || isFiniteNumber(value);
}

function isTarget(payload: unknown): payload is TargetPayload {
  return isRecord(payload) && typeof payload.target === "string";
}

function isTurnRetry(payload: unknown): payload is TurnRetryPayload {
  return (
    isRecord(payload) &&
    isFiniteNumber(payload.turn) &&
    isFiniteNumber(payload.attempt) &&
    typeof payload.reason === "string"
  );
}

function isTerminal(payload: unknown): payload is TerminalPayload {
  if (!isRecord(payload) || typeof payload.outcome !== "string") return false;
  if (payload.health !== undefined && !isRecord(payload.health)) return false;
  return true;
}

/**
 * A child session recompiling a parent leaf. The kernel does not copy the
 * parent path onto the child.
 */
type SubtaskPayload = {
  parentSessionId: string;
  parentLeaf: string;
  query: string;
  budget: number;
  readOnly: boolean;
};

function isSubtask(payload: unknown): payload is SubtaskPayload {
  return (
    isRecord(payload) &&
    typeof payload.parentSessionId === "string" &&
    typeof payload.parentLeaf === "string" &&
    typeof payload.query === "string" &&
    isFiniteNumber(payload.budget) &&
    typeof payload.readOnly === "boolean"
  );
}

registerEntryKind<Message>({
  kind: "message",
  doc: "Conversation message. Payload is { role, content }.",
  validate: isMessage,
});

registerEntryKind<RequestHeaderPayload>({
  kind: "request_header",
  doc: "Request envelope, including the call's contextWindow and pricing (null when uncatalogued). system and tools are stored in full only when their hash differs from the previous request_header, or there is none.",
  validate: isRequestHeader,
});

registerEntryKind<ViewEpochPayload>({
  kind: "view_epoch",
  doc: "A new view epoch. The previous prefix is left intact.",
  validate: isViewEpoch,
});

registerEntryKind<ModelChangePayload>({
  kind: "model_change",
  doc: "Model switch recorded as an entry. History is not rewritten.",
  validate: isModelChange,
});

registerEntryKind<ManifestPayload>({
  kind: "manifest",
  doc: "Compile manifest snapshot. The kernel does not interpret claims, chunks, or entryScored.",
  validate: isManifest,
});

registerEntryKind<TargetPayload>({
  kind: "undo",
  doc: "Undo recorded as an entry pointing at an earlier id. History is not rewritten.",
  validate: isTarget,
});

registerEntryKind<TargetPayload>({
  kind: "leaf",
  doc: "Moves the leaf pointer to an existing entry. Does not copy history.",
  validate: isTarget,
});

registerEntryKind<TurnRetryPayload>({
  kind: "turn_retry",
  doc: "Whole-turn resend after a transport error or a transient in-stream error event. Not model-visible. Payload is { turn, attempt, reason }.",
  validate: isTurnRetry,
});

registerEntryKind<TerminalPayload>({
  kind: "terminal",
  doc: "Terminal outcome of a run. Optional usage and error stay opaque.",
  validate: isTerminal,
});

registerEntryKind<SubtaskPayload>({
  kind: "subtask",
  doc: "Child session pointer. Names the parent leaf it recompiles. History is not copied.",
  validate: isSubtask,
});
