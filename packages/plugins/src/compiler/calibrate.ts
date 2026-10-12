/**
 * Anchor token estimates on the previous provider usage.
 *
 * Contract: `measuredInput` is the latest assistant message's
 * `details.usage` input total (`inputOther + inputCacheRead +
 * inputCacheCreation`). `calibrationFor` pairs that total with the previous
 * manifest's `budget.used`. `overheadTokens` is the first computable turn
 * of the current epoch: that turn's measured input minus its
 * `budget.used`, floored at 0. When that turn has no usage, it is the most
 * recent computable value on the path. Later turns set `ratio` to
 * `(measured − overheadTokens) / estimated`. No usage, or a zero estimate,
 * yields undefined; callers then use ratio 1 and overhead 0. Callers scale
 * block tokens by the raw ratio, including when that ratio is above 1.
 * An overhead below 0 is stored as 0: a negative overhead makes new-view
 * tokens negative and the cost check reopens immediately.
 * `expectedFrozenPrefix` reads only the previous manifest's own
 * calibration, never the current epoch's re-anchored overhead.
 *
 * Intentionally not in the Log: these functions. The manifest stores the
 * resulting calibration object.
 */

import {
  epochSpan,
  finiteOr,
  isFiniteNumber,
  isMessage,
  isRecord,
  latestViewEpoch,
  type StoredEntry,
} from "@sage/core";

/**
 * How this turn's estimates map to measured input tokens. Stored on the
 * manifest as `calibration`.
 */
export type Calibration = {
  measured: number;
  estimated: number;
  /** Fixed overhead: epoch's first measured input minus its `budget.used`. */
  overheadTokens: number;
  ratio: number;
};

/** Input total of the latest assistant message's usage, if it has one. */
export function measuredInput(
  path: readonly StoredEntry[],
): number | undefined {
  const usage = latestUsage(path);
  if (!usage) return undefined;
  return inputTotal(usage);
}

/** Cache-read tokens of the latest assistant usage, or 0. */
export function measuredCacheRead(path: readonly StoredEntry[]): number {
  const usage = latestUsage(path);
  return usage ? finiteOr(usage.inputCacheRead, 0) : 0;
}

/**
 * Pairs the latest measured input with the previous manifest's estimate.
 * Undefined without usage or with a zero estimate.
 */
export function calibrationFor(
  path: readonly StoredEntry[],
): Calibration | undefined {
  const measured = measuredInput(path);
  const estimated = previousEstimate(path);
  if (measured === undefined || estimated === undefined || estimated === 0) {
    return undefined;
  }
  const overheadTokens = anchorOverhead(path) ?? 0;
  return {
    measured,
    estimated,
    overheadTokens,
    ratio: (measured - overheadTokens) / estimated,
  };
}

/**
 * The frozen-prefix read the previous manifest predicted:
 * `overheadTokens + ratio × budget.frozenTokens` from that manifest's own
 * calibration. A manifest without calibration uses overhead 0 and ratio 1;
 * a calibration without `overheadTokens` uses overhead 0. A later epoch's
 * re-anchored overhead is not applied to the earlier freeze. 0 without a
 * previous manifest.
 */
export function expectedFrozenPrefix(path: readonly StoredEntry[]): number {
  const manifest = previousManifest(path);
  if (!manifest || !isRecord(manifest.payload)) return 0;
  const prior = storedCalibration(manifest.payload);
  return prior.overheadTokens + frozenTokens(manifest.payload) * prior.ratio;
}

function anchorOverhead(path: readonly StoredEntry[]): number | undefined {
  const epoch = latestViewEpoch(path);
  const span = epochSpan(path, epoch) ?? {
    start: path.length,
    end: path.length,
  };
  const first = firstOverhead(path.slice(span.start, span.end));
  if (first !== undefined) return first;
  return latestOverhead(path.slice(0, span.end));
}

function firstOverhead(path: readonly StoredEntry[]): number | undefined {
  return overheadFromPairs(path, false);
}

function latestOverhead(path: readonly StoredEntry[]): number | undefined {
  return overheadFromPairs(path, true);
}

function overheadFromPairs(
  path: readonly StoredEntry[],
  latest: boolean,
): number | undefined {
  let pending: number | undefined;
  let found: number | undefined;
  for (const entry of path) {
    if (entry.kind === "manifest") {
      pending = budgetUsed(entry);
      continue;
    }
    if (entry.kind !== "message" || !isAssistant(entry)) continue;
    const usage = entry.details?.usage;
    if (!isRecord(usage) || pending === undefined || pending === 0) continue;
    const measured = inputTotal(usage);
    if (measured === undefined) continue;
    found = Math.max(0, measured - pending);
    if (!latest) return found;
    pending = undefined;
  }
  return found;
}

function previousEstimate(path: readonly StoredEntry[]): number | undefined {
  const manifest = previousManifest(path);
  if (!manifest) return undefined;
  return budgetUsed(manifest);
}

function previousManifest(
  path: readonly StoredEntry[],
): StoredEntry | undefined {
  for (let index = path.length - 1; index >= 0; index -= 1) {
    const entry = path[index];
    if (entry?.kind === "manifest") return entry;
  }
  return undefined;
}

function budgetUsed(entry: StoredEntry): number | undefined {
  if (!isRecord(entry.payload)) return undefined;
  const budget = entry.payload.budget;
  if (typeof budget === "number") return finiteOr(budget, undefined);
  if (!isRecord(budget)) return undefined;
  return finiteOr(budget.used, undefined);
}

function frozenTokens(payload: Record<string, unknown>): number {
  const budget = payload.budget;
  if (!isRecord(budget)) return 0;
  return finiteOr(budget.frozenTokens, 0);
}

function storedCalibration(payload: Record<string, unknown>): {
  overheadTokens: number;
  ratio: number;
} {
  const calibration = payload.calibration;
  if (!isRecord(calibration)) return { overheadTokens: 0, ratio: 1 };
  return {
    overheadTokens: finiteOr(calibration.overheadTokens, 0),
    ratio: finiteOr(calibration.ratio, 1),
  };
}

function latestUsage(
  path: readonly StoredEntry[],
): Record<string, unknown> | undefined {
  for (let index = path.length - 1; index >= 0; index -= 1) {
    const entry = path[index];
    if (!entry || !isAssistant(entry)) continue;
    const usage = entry.details?.usage;
    return isRecord(usage) ? usage : undefined;
  }
  return undefined;
}

function isAssistant(entry: StoredEntry): boolean {
  return (
    entry.kind === "message" &&
    isMessage(entry.payload) &&
    entry.payload.role === "assistant"
  );
}

/**
 * Strict input total: undefined when any input count is missing or not
 * finite, where `readTokenCounts` would read it as 0 and skew the ratio.
 */
function inputTotal(usage: Record<string, unknown>): number | undefined {
  const { inputOther, inputCacheRead, inputCacheCreation } = usage;
  if (
    !isFiniteNumber(inputOther) ||
    !isFiniteNumber(inputCacheRead) ||
    !isFiniteNumber(inputCacheCreation)
  ) {
    return undefined;
  }
  return inputOther + inputCacheRead + inputCacheCreation;
}
