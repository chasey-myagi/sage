/**
 * Reads the user's providers file, `~/.sage/providers.json`, and hands the
 * validated presets and catalog entries to the provider layer.
 *
 * Contract: a missing file means no user providers. An unreadable file,
 * invalid JSON, or a shape `parseLocalProviders` rejects comes back as one
 * stderr line naming the file path and the offending field, and the command
 * exits 2; nothing is silently skipped. Only the user-level file is read, never one under the
 * workspace: a project file is easy to commit and would publish a private
 * gateway. `sage bench run` children inherit the parent's env, hence its
 * HOME, so they read the same file.
 *
 * Intentionally not in the Log: the file path and its presets. Prices
 * reach the Log through request_header.pricing.
 */

import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";
import { errorText, SAGE_DIR_NAME } from "@sage/core";
import {
  type LocalProviders,
  NO_LOCAL_PROVIDERS,
  parseLocalProviders,
} from "@sage/provider";

/** File name of the providers file under `<home>/.sage`. */
const LOCAL_PROVIDERS_FILE = "providers.json";

/**
 * `Pricing.source` for entries that omit one. Spelled with `~` so the Log
 * never carries the user's home path.
 */
const DEFAULT_SOURCE = path.posix.join(
  "~",
  SAGE_DIR_NAME,
  LOCAL_PROVIDERS_FILE,
);

/** The user's providers and the home they came from, or the stderr line. */
type LoadedProviders =
  | { ok: true; home: string; local: LocalProviders }
  | { ok: false; message: string };

/**
 * Loads `<home>/.sage/providers.json`, where home is `env.HOME` when set,
 * else `os.homedir()`. Reading HOME from the env the CLI was given keeps
 * the parent and its bench children on the same file. A missing file
 * yields {@link NO_LOCAL_PROVIDERS}; any other failure is a value whose
 * `message` names the file and the field. Never throws.
 */
export function loadLocalProviders(env: NodeJS.ProcessEnv): LoadedProviders {
  const home =
    typeof env.HOME === "string" && env.HOME.length > 0 ? env.HOME : homedir();
  const file = path.join(home, SAGE_DIR_NAME, LOCAL_PROVIDERS_FILE);
  const text = readText(file);
  if (!text.ok) return text;
  if (text.value === undefined) {
    return { ok: true, home, local: NO_LOCAL_PROVIDERS };
  }
  const json = parseJson(text.value, file);
  if (!json.ok) return json;
  const parsed = parseLocalProviders(json.value, {
    defaultSource: DEFAULT_SOURCE,
  });
  if (!parsed.ok) {
    return {
      ok: false,
      message: `${file}: ${parsed.field}: ${parsed.message}`,
    };
  }
  return { ok: true, home, local: parsed.value };
}

/** The file's text, undefined when it does not exist, or why it is unreadable. */
function readText(
  file: string,
): { ok: true; value: string | undefined } | { ok: false; message: string } {
  try {
    return { ok: true, value: readFileSync(file, "utf8") };
  } catch (error) {
    if (isNotFound(error)) return { ok: true, value: undefined };
    return { ok: false, message: `${file}: cannot read: ${errorText(error)}` };
  }
}

function parseJson(
  text: string,
  file: string,
): { ok: true; value: unknown } | { ok: false; message: string } {
  try {
    return { ok: true, value: JSON.parse(text) };
  } catch (error) {
    return { ok: false, message: `${file}: invalid JSON: ${errorText(error)}` };
  }
}

function isNotFound(error: unknown): boolean {
  return error instanceof Error && "code" in error && error.code === "ENOENT";
}
