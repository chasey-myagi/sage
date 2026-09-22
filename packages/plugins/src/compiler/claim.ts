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

import type { ViewContext } from "@sage/core";

export type Tier = "none" | "line" | "summary" | "full";

export type Placement = "epoch-head" | "tail" | "tail-if-changed";

export type Claim = {
  id: string;
  priority: number;
  pinned?: boolean;
  placement: Placement;
  /** Rendered text for each usable tier. Absent tiers cannot be selected. */
  tiers: Partial<Record<Exclude<Tier, "none">, string>>;
};

export type ClaimProvider = (ctx: ViewContext) => Claim[] | Promise<Claim[]>;
