/**
 * Subtask presets discovered from markdown files.
 *
 * Contract: `loadPresets` reads `<cwd>/.sage/subtasks/*.md` and, when `home`
 * is set, `<home>/.sage/subtasks/*.md`. The preset name is the file stem. A
 * project file replaces the user file with the same stem. The frontmatter
 * block recognizes only `name`, `budget`, `readOnly`, and `model`
 * (`provider/model`). `name` does not rename the preset. The body is the
 * query template. `{{args.xxx}}` is filled when a spawn tool renders it. A
 * missing arg becomes an empty string.
 *
 * Intentionally not in the Log: the file list and the template source. The
 * rendered query is stored on the child `subtask` entry.
 */

import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { SAGE_DIR_NAME } from "@sage/core";
import type { ModelRef } from "@sage/provider/contract";

/** A subtask template loaded from `.sage/subtasks/<name>.md`. */
export type SubtaskPreset = {
  name: string;
  query: string;
  budget?: number;
  readOnly?: boolean;
  model?: ModelRef;
};

/** Test seam. Production uses `node:fs`. */
export type PresetFs = {
  readdirSync(dir: string): string[];
  readFileSync(file: string, encoding: "utf8"): string;
};

/**
 * Reads the user and project preset directories. A missing directory or an
 * unreadable file is skipped, not thrown.
 */
export function loadPresets(opts: {
  cwd: string;
  home?: string;
  fs?: PresetFs;
}): Map<string, SubtaskPreset> {
  const fs = opts.fs ?? { readdirSync, readFileSync };
  const presets = new Map<string, SubtaskPreset>();
  if (opts.home) {
    readDir(fs, path.join(opts.home, SAGE_DIR_NAME, "subtasks"), presets);
  }
  readDir(fs, path.join(opts.cwd, SAGE_DIR_NAME, "subtasks"), presets);
  return presets;
}

/** Fills `{{args.name}}` placeholders; a missing arg becomes "". */
export function renderPresetQuery(
  template: string,
  args: Record<string, string>,
): string {
  return template.replace(
    /\{\{args\.([A-Za-z_][A-Za-z0-9_]*)\}\}/g,
    (_match, key: string) => args[key] ?? "",
  );
}

function readDir(
  fs: PresetFs,
  dir: string,
  into: Map<string, SubtaskPreset>,
): void {
  let names: string[];
  try {
    names = fs.readdirSync(dir);
  } catch {
    return;
  }
  for (const name of names) {
    if (!name.endsWith(".md")) continue;
    const stem = name.slice(0, -".md".length);
    if (stem.length === 0) continue;
    let text: string;
    try {
      text = fs.readFileSync(path.join(dir, name), "utf8");
    } catch {
      continue;
    }
    into.set(stem, parsePreset(stem, text));
  }
}

function parsePreset(name: string, text: string): SubtaskPreset {
  const normalized = text.replace(/\r\n/g, "\n");
  const matched = /^---\n([\s\S]*?)\n---\n?([\s\S]*)$/.exec(normalized);
  const body = (matched ? matched[2] : normalized).trim();
  const preset: SubtaskPreset = { name, query: body };
  if (!matched?.[1]) return preset;
  for (const line of matched[1].split("\n")) {
    const colon = line.indexOf(":");
    if (colon <= 0) continue;
    const key = line.slice(0, colon).trim();
    const value = stripQuotes(line.slice(colon + 1).trim());
    if (key === "budget") {
      const budget = Number(value);
      if (Number.isFinite(budget)) preset.budget = budget;
      continue;
    }
    if (key === "readOnly") {
      if (value === "true") preset.readOnly = true;
      if (value === "false") preset.readOnly = false;
      continue;
    }
    if (key === "model") {
      const model = readModel(value);
      if (model) preset.model = model;
    }
  }
  return preset;
}

function readModel(value: string): ModelRef | undefined {
  const slash = value.indexOf("/");
  if (slash <= 0 || slash === value.length - 1) return undefined;
  return {
    provider: value.slice(0, slash),
    model: value.slice(slash + 1),
  };
}

function stripQuotes(value: string): string {
  if (
    (value.startsWith('"') && value.endsWith('"')) ||
    (value.startsWith("'") && value.endsWith("'"))
  ) {
    return value.slice(1, -1);
  }
  return value;
}
