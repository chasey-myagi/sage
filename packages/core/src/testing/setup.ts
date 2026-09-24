/**
 * Vitest setup. Runs before every test file.
 *
 * Guarantees: strip every `*_API_KEY` from `process.env`, then refuse to
 * continue if any remain. Replace global `fetch` with a stub that throws.
 * No test in this process can call a provider.
 *
 * Intentionally not in the Log: the stripped environment and the fetch stub.
 * They are process guards, not replayable facts.
 */

import { stripSecrets } from "./secrets.js";

stripSecrets(process.env);

const leaked = Object.keys(process.env).filter((key) =>
  key.endsWith("_API_KEY"),
);
if (leaked.length > 0) {
  throw new Error(
    `refusing to run tests with API keys still set: ${leaked.join(", ")}`,
  );
}

globalThis.fetch = (async () => {
  throw new Error(
    "network disabled: tests must not call fetch. Replay uses the faux adapter.",
  );
}) as typeof fetch;
