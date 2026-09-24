/**
 * Claim submitted by a plugin. The compiler allocates tiers; plugins do not
 * append text themselves.
 *
 * Contract: `tiers` holds the rendered text of each available tier. A missing
 * tier cannot be chosen. `none` is the floor and has no text.
 *
 * Intentionally not in the Log: the provider function and the live claim
 * object. The kernel logs the allocated record on the manifest entry.
 */

import type { Tier, ViewContext } from "@sage/core";

/** Core's `Tier`, re-exported so claim providers import one module. */
export type { Tier };

/**
 * Where a claim renders: `epoch-head` is frozen for the epoch at the start
 * of the messages; `tail` is recomputed every turn after the last message;
 * `tail-if-changed` is a tail claim whose manifest reason says whether its
 * text changed since the previous manifest.
 */
export type Placement = "epoch-head" | "tail" | "tail-if-changed";

/**
 * A claim is a plugin's request for view space: who it is, how important,
 * where it renders, and the text of each tier it can shrink to. The
 * compiler decides the tier; a plugin never writes view text directly.
 */
export type Claim = {
  id: string;
  priority: number;
  pinned?: boolean;
  placement: Placement;
  /** Rendered text for each usable tier. Absent tiers cannot be selected. */
  tiers: Partial<Record<Exclude<Tier, "none">, string>>;
};

/**
 * Returns this turn's claims. A provider that throws becomes one manifest
 * row and does not stop the compile.
 */
export type ClaimProvider = (ctx: ViewContext) => Claim[] | Promise<Claim[]>;
