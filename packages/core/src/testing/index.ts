/**
 * Test harness public surface: `@sage/core/testing`, not part of the kernel
 * index. The faux provider, replay, snapshots, the credential strip, the
 * scripted SSE `fetch`, and the `LogFs` double live here so a test can run without an API key or a real
 * disk fault; every package's tests take their doubles from here.
 *
 * Guarantees: `setup.ts` is a vitest setupFile only and is not re-exported,
 * so importing this module does not strip the caller's environment.
 *
 * Intentionally not in the Log: harness helpers, the record switch, and
 * expected-file diffs. A recorded `session.jsonl` is itself a Log.
 */

export { soleChildRun } from "./child-runs.js";
export {
  createFaux,
  FAUX_CONTEXT_WINDOW,
  FAUX_MODEL,
  FAUX_SYSTEM,
  FAUX_UNIT_USAGE,
  FAUX_USAGE,
  fauxAdapter,
  stopScript,
  toolScript,
} from "./faux.js";
export { expectFile } from "./files.js";
export { nodeLogFs } from "./log-fs.js";
export { renderLlmInput } from "./render.js";
export { loadCase, replaySession } from "./replay.js";
export { stripSecrets } from "./secrets.js";
export { type SseFetchStep, scriptedSseFetch } from "./sse-fetch.js";
export { pathlessTool, pathTool } from "./tools.js";
export { recordViews } from "./views.js";
