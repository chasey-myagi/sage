/**
 * Workspace file walk shared by find and grep's in-process searcher.
 *
 * Contract: lists regular files under a root without following symlinks,
 * skipping any entry named in `SKIPPED_DIR_NAMES`. A root that is a file is
 * listed as itself. A root or directory that cannot be read contributes
 * nothing. It does not throw.
 *
 * Intentionally not in the Log: the walk. Callers log only their tool
 * result.
 */

import { type Dirent, readdirSync, statSync } from "node:fs";
import path from "node:path";
import { SKIPPED_DIR_NAMES } from "./path-safety.js";

/**
 * Absolute paths of the regular files under `root`, in walk order, not
 * sorted. Callers sort what they report.
 */
export function walkWorkspaceFiles(root: string): string[] {
  const files: string[] = [];
  let rootIsFile: boolean;
  try {
    rootIsFile = statSync(root).isFile();
  } catch {
    // A missing or unreadable root lists nothing; the tool reports no match.
    return files;
  }
  if (rootIsFile) return [root];
  const stack = [root];
  while (stack.length > 0) {
    const current = stack.pop();
    if (current === undefined) break;
    let entries: Dirent[];
    try {
      entries = readdirSync(current, { withFileTypes: true });
    } catch {
      // An unreadable directory is skipped; its siblings are still walked.
      continue;
    }
    for (const entry of entries) {
      if (SKIPPED_DIR_NAMES.has(entry.name)) continue;
      if (entry.isSymbolicLink()) continue;
      const abs = path.join(current, entry.name);
      if (entry.isDirectory()) stack.push(abs);
      else if (entry.isFile()) files.push(abs);
    }
  }
  return files;
}
