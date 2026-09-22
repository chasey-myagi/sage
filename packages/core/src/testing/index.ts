/**
 * Test harness public surface.
 *
 * Contract: this subpath is `@sage/core/testing`. It is not part of the kernel
 * index. Replay, snapshots, and the credential strip live here so a test can
 * run without an API key. `setup.ts` is a vitest setupFile only and is not
 * re-exported, so importing this module does not strip the caller environment.
 *
 * Intentionally not in the Log: harness helpers, the record switch, and
 * expected-file diffs. A recorded `session.jsonl` is itself a Log.
 */

export { fauxAdapter } from "./faux.js";
export { expectFile } from "./files.js";
export { renderLlmInput } from "./render.js";
export { loadCase, replaySession } from "./replay.js";
export { stripSecrets } from "./secrets.js";
