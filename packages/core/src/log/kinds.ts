/**
 * Kernel entry kinds. Registered when this module is imported.
 *
 * Contract: validators check shape only. None of these kinds ship a reducer
 * yet. `leaf` is kernel semantics (it moves the pointer); the other kinds are
 * recorded and left for later folds.
 *
 * `message` payload is `Message` from `@sage/provider/contract` (type-only).
 * The validator checks that `role` is a string and `content` is present.
 * Do not value-import `@sage/provider`.
 *
 * `request_header.system` and `request_header.tools` are present only when
 * they differ from the default. The kernel does not decide that; it only
 * checks the shape.
 *
 * `manifest.budget` is either a finite number or an object with finite
 * `total`, `used`, and `overrun`. `frozenTokens` and `staleOmittable`, when
 * present, are finite numbers. Other fields are left unchecked. The kernel
 * does not interpret the object.
 * `epochDecision`, `calibration`, and `cache` are optional objects. When
 * `epochDecision` is present, `remainingTurns` is a finite number. The
 * kernel checks that shape and does not interpret the decision. When
 * `calibration` is present, `measured`, `estimated`, `ratio`, and `head`
 * are finite numbers if they are present. Older manifests may omit `head`.
 *
 * `terminal.health`, when present, is an object. The kernel does not read
 * its fields. A missing health is valid. A non-object health is rejected.
 *
 * `subtask` records that a child session recompiles a parent leaf. The
 * payload is `{ parentSessionId, parentLeaf, query, budget, readOnly }`.
 * The kernel does not copy the parent path onto the child.
 *
 * Intentionally not in the Log: reducer functions, the registry, and any
 * high-water mark.
 */

import type { Message } from "@sage/provider/contract";
import { registerEntryKind } from "./registry.js";
import { isRecord, type ModelRef } from "./types.js";

export type RequestHeaderPayload = {
  provider: string;
  model: string;
  systemPromptHash: string;
  toolsHash: string;
  sampling: Record<string, unknown>;
  adapterDefaults: Record<string, unknown>;
  system?: string;
  tools?: unknown;
};

export type ViewEpochPayload = {
  epoch: number;
  reason: string;
};

export type ModelChangePayload = {
  from: ModelRef;
  to: ModelRef;
  reason: string;
};

export type ManifestPayload = {
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
};

export type TargetPayload = {
  target: string;
};

export type TerminalPayload = {
  outcome: string;
  usage?: unknown;
  error?: unknown;
  /** Opaque run-health object. Counts only; the kernel does not read it. */
  health?: Record<string, unknown>;
};

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
  if (payload.system !== undefined && typeof payload.system !== "string") {
    return false;
  }
  return true;
}

function isViewEpoch(payload: unknown): payload is ViewEpochPayload {
  return (
    isRecord(payload) &&
    typeof payload.epoch === "number" &&
    Number.isFinite(payload.epoch) &&
    typeof payload.reason === "string"
  );
}

function isModelRef(value: unknown): value is ModelRef {
  return (
    isRecord(value) &&
    typeof value.provider === "string" &&
    typeof value.model === "string"
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
  if (typeof payload.epoch !== "number" || !Number.isFinite(payload.epoch)) {
    return false;
  }
  if (typeof payload.turn !== "number" || !Number.isFinite(payload.turn)) {
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
  return (
    Array.isArray(payload.claims) &&
    Array.isArray(payload.chunks) &&
    Array.isArray(payload.anomalies)
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
    isOptionalFinite(value.head)
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
  if (typeof value === "number") return Number.isFinite(value);
  if (!isRecord(value)) return false;
  if (!isFiniteNumber(value.total)) return false;
  if (!isFiniteNumber(value.used)) return false;
  if (!isFiniteNumber(value.overrun)) return false;
  if (!isOptionalFinite(value.frozenTokens)) return false;
  if (!isOptionalFinite(value.staleOmittable)) return false;
  return true;
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function isOptionalFinite(value: unknown): boolean {
  return value === undefined || isFiniteNumber(value);
}

function isTarget(payload: unknown): payload is TargetPayload {
  return isRecord(payload) && typeof payload.target === "string";
}

function isTerminal(payload: unknown): payload is TerminalPayload {
  if (!isRecord(payload) || typeof payload.outcome !== "string") return false;
  if (payload.health !== undefined && !isRecord(payload.health)) return false;
  return true;
}

export type SubtaskPayload = {
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
    typeof payload.budget === "number" &&
    Number.isFinite(payload.budget) &&
    typeof payload.readOnly === "boolean"
  );
}

registerEntryKind<Message>({
  kind: "message",
  persisted: true,
  doc: "Conversation message. Payload is { role, content }.",
  validate: isMessage,
});

registerEntryKind<RequestHeaderPayload>({
  kind: "request_header",
  persisted: true,
  doc: "Request envelope. system and tools are stored only when they differ from the default.",
  validate: isRequestHeader,
});

registerEntryKind<ViewEpochPayload>({
  kind: "view_epoch",
  persisted: true,
  doc: "A new view epoch. The previous prefix is left intact.",
  validate: isViewEpoch,
});

registerEntryKind<ModelChangePayload>({
  kind: "model_change",
  persisted: true,
  doc: "Model switch recorded as an entry. History is not rewritten.",
  validate: isModelChange,
});

registerEntryKind<ManifestPayload>({
  kind: "manifest",
  persisted: true,
  doc: "Compile manifest snapshot. The kernel does not interpret claims or chunks.",
  validate: isManifest,
});

registerEntryKind<TargetPayload>({
  kind: "undo",
  persisted: true,
  doc: "Undo recorded as an entry pointing at an earlier id. History is not rewritten.",
  validate: isTarget,
});

registerEntryKind<TargetPayload>({
  kind: "leaf",
  persisted: true,
  doc: "Moves the leaf pointer to an existing entry. Does not copy history.",
  validate: isTarget,
});

registerEntryKind<TerminalPayload>({
  kind: "terminal",
  persisted: true,
  doc: "Terminal outcome of a run. Optional usage and error stay opaque.",
  validate: isTerminal,
});

registerEntryKind<SubtaskPayload>({
  kind: "subtask",
  persisted: true,
  doc: "Child session pointer. Names the parent leaf it recompiles. History is not copied.",
  validate: isSubtask,
});
