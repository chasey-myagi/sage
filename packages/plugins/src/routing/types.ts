/**
 * Route policy surface. Called by the compiler only when an epoch rebuilds.
 *
 * Contract: a policy returns the next model and a reason, or undefined to
 * keep the current one. `sensitivePolicy` is a candidate filter, not a
 * policy. `viewChunks` carry the new view's tiers plus accesses from the
 * chunk index. Those accesses are not a log field.
 *
 * Intentionally not in the Log: policy functions and the candidate list.
 * The kernel records a switch as `model_change`. The compiler copies the
 * reason onto the manifest.
 */

import type { Access, ChunkRecord } from "@sage/core";
import type { CatalogEntry, ModelRef } from "@sage/provider";
import type { EpochDecision } from "../compiler/decide.js";

export type RouteInput = {
  epoch: number;
  current: ModelRef;
  candidates: readonly CatalogEntry[];
  viewChunks: readonly (ChunkRecord & { accesses?: Access[] })[];
  decision: EpochDecision;
  budget: number;
};

export type RouteResult = { model: ModelRef; reason: string } | undefined;

export type RoutePolicy = (input: RouteInput) => RouteResult;
