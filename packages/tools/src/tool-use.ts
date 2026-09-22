/**
 * Binds the current tool-use id for spill filenames.
 *
 * Contract: HarnessTool.execute does not receive the tool-use id. The
 * session wraps execute so bash can name `<cwd>/.sage/tool-output/<id>.txt`
 * after the call that produced the output.
 *
 * Intentionally not in the Log: the async-local binding. The spill path is
 * model-visible only once it appears in the tool result.
 */

import { AsyncLocalStorage } from "node:async_hooks";

const storage = new AsyncLocalStorage<string>();

export function currentToolUseId(): string | undefined {
  return storage.getStore();
}

export function withToolUseId<T>(id: string, fn: () => Promise<T>): Promise<T> {
  return storage.run(id, fn);
}
