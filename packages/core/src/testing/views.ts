/**
 * View recorder: wraps a ViewProvider and keeps every view it compiles, so a
 * test asserts on the compiled view instead of the provider request.
 *
 * Guarantees: each recorded view is a structured clone taken after the inner
 * provider returns, so later edits by the kernel or observers never change
 * it. The wrapper returns the inner view unchanged.
 *
 * Intentionally not in the Log: the recorded views. The kernel logs each
 * view's request entries and manifest itself.
 */

import { passthroughView } from "../view/passthrough.js";
import type { CompiledView, ViewProvider } from "../view/types.js";

/**
 * Wraps `inner` (default `passthroughView`). `views` fills in call order,
 * one structured clone per `view()` call, including calls from concurrent
 * sessions sharing the wrapper. Pass `views` to share one list across
 * several wrappers, such as one per child session.
 */
export function recordViews(
  inner: ViewProvider = passthroughView,
  views: CompiledView[] = [],
): ViewProvider & { views: CompiledView[] } {
  return {
    views,
    async view(ctx) {
      const compiled = await inner.view(ctx);
      views.push(structuredClone(compiled));
      return compiled;
    },
  };
}
