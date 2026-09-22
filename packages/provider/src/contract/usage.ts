/**
 * Usage quadruple shared by every adapter.
 *
 * Deliberately not a Log entry by itself. The assistant entry that lands
 * in the Log copies this object (#4). `raw` is the provider payload escape
 * hatch and is not a second source of the four numbers.
 */

export type Usage = {
  inputOther: number;
  inputCacheRead: number;
  inputCacheCreation: number;
  output: number;
  raw?: unknown;
};
