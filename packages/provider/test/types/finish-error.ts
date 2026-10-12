// Compile-time checks on `FinishError`. Vitest does not type-check and the
// package tsconfig covers only `src/`, so this directory is its own project
// in the root `tsc -b` graph: an unused `@ts-expect-error` here fails
// `pnpm check:types`.

import type { FinishError } from "../../src/contract/stream.js";

const event = { transient: true };

export const accepted: FinishError[] = [
  { message: "fetch failed", cause: "transport" },
  { message: "HTTP 500", cause: "http", status: 500, raw: {} },
  { message: "Overloaded", cause: "event", event, raw: {} },
  { message: "decode failed", cause: "other" },
];

export const transportWithStatus: FinishError = {
  message: "x",
  cause: "transport",
  // @ts-expect-error a transport failure carries no HTTP status
  status: 502,
};
export const httpWithTransportFlag: FinishError = {
  message: "x",
  cause: "http",
  status: 502,
  // @ts-expect-error there is no separate transport flag to set
  transport: true,
};
// @ts-expect-error an HTTP failure always carries its status
export const httpWithoutStatus: FinishError = { message: "x", cause: "http" };
export const httpWithEvent: FinishError = {
  message: "x",
  cause: "http",
  status: 500,
  // @ts-expect-error an HTTP failure is not an in-stream event
  event,
};
// @ts-expect-error an in-stream event always carries its classification
export const eventWithoutClass: FinishError = { message: "x", cause: "event" };
export const otherWithStatus: FinishError = {
  message: "x",
  cause: "other",
  // @ts-expect-error an unclassified failure carries no status
  status: 500,
};
// @ts-expect-error every failure names its cause
export const withoutCause: FinishError = { message: "x" };

/** Fields are readable only after narrowing on `cause`. */
export function statusOf(error: FinishError): number | undefined {
  // @ts-expect-error `status` is not on every cause
  void error.status;
  // @ts-expect-error `event` is not on every cause
  void error.event;
  return error.cause === "http" ? error.status : undefined;
}
