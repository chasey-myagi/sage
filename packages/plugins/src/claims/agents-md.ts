/**
 * AGENTS.md claims.
 *
 * Contract: the workspace file, when present, is an epoch-head claim at
 * priority 80 (overridable). full is the file text. summary is the first 40
 * lines, and a following ellipsis when the file is longer. A nested
 * AGENTS.md is a tail-if-changed claim at priority 70, id `agents-md:<dir>`,
 * and only when some assistant tool_use input has a string field named
 * `path` inside that directory. Text is escaped before it is returned.
 *
 * Intentionally not in the Log: file bytes that were not selected into a
 * tier. Selected text reaches the Log only as manifest claim content.
 */

import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import type { ViewContext } from "@sage/core";
import type { Claim, ClaimProvider } from "../compiler/claim.js";
import { escapeClaimText } from "./escape.js";

const SUMMARY_LINES = 40;

type AgentsFs = {
  readFileSync: (file: string, encoding: "utf8") => string;
  existsSync: (file: string) => boolean;
};

const nodeFs: AgentsFs = {
  existsSync,
  readFileSync: (file, encoding) => readFileSync(file, encoding),
};

export function agentsMd(opts: {
  cwd: string;
  fs?: AgentsFs;
  priority?: number;
}): ClaimProvider {
  const fs = opts.fs ?? nodeFs;
  const cwd = path.resolve(opts.cwd);

  return (ctx) => {
    const claims: Claim[] = [];
    const root = readAgents(fs, path.join(cwd, "AGENTS.md"));
    if (root !== undefined) {
      claims.push({
        id: "agents-md",
        priority: opts.priority ?? 80,
        placement: "epoch-head",
        tiers: tiersFor(root),
      });
    }
    const nested = new Map<string, string>();
    for (const dir of touchedDirs(cwd, ctx)) {
      const rel = toPosix(path.relative(cwd, dir));
      if (!rel || rel.startsWith("..")) continue;
      const text = readAgents(fs, path.join(dir, "AGENTS.md"));
      if (text === undefined) continue;
      nested.set(rel, text);
    }
    for (const rel of [...nested.keys()].sort((a, b) => a.localeCompare(b))) {
      const text = nested.get(rel);
      if (text === undefined) continue;
      claims.push({
        id: `agents-md:${rel}`,
        priority: 70,
        placement: "tail-if-changed",
        tiers: tiersFor(text),
      });
    }
    return claims;
  };
}

function tiersFor(text: string): { full: string; summary: string } {
  return { full: escapeClaimText(text), summary: summaryOf(text) };
}

function summaryOf(text: string): string {
  const body = text.endsWith("\n") ? text.slice(0, -1) : text;
  if (body === "") return escapeClaimText(text);
  const lines = body.split("\n");
  if (lines.length <= SUMMARY_LINES) return escapeClaimText(text);
  return escapeClaimText(`${lines.slice(0, SUMMARY_LINES).join("\n")}\n…`);
}

function readAgents(fs: AgentsFs, file: string): string | undefined {
  if (!fs.existsSync(file)) return undefined;
  return fs.readFileSync(file, "utf8");
}

function touchedDirs(cwd: string, ctx: ViewContext): string[] {
  const dirs: string[] = [];
  const seen = new Set<string>();
  for (const raw of pathFields(ctx)) {
    const abs = path.resolve(cwd, raw);
    let current = abs;
    while (isInside(cwd, current)) {
      if (seen.has(current)) break;
      seen.add(current);
      dirs.push(current);
      const parent = path.dirname(current);
      if (parent === current) break;
      current = parent;
    }
  }
  return dirs;
}

function pathFields(ctx: ViewContext): string[] {
  const found: string[] = [];
  for (const entry of ctx.path) {
    if (entry.kind !== "message") continue;
    const payload = entry.payload;
    if (!isRecord(payload) || payload.role !== "assistant") continue;
    if (!Array.isArray(payload.content)) continue;
    for (const block of payload.content) {
      if (!isRecord(block) || block.type !== "tool_use") continue;
      collectPaths(block.input, found);
    }
  }
  return found;
}

function collectPaths(value: unknown, out: string[]): void {
  if (Array.isArray(value)) {
    for (const item of value) collectPaths(item, out);
    return;
  }
  if (!isRecord(value)) return;
  for (const [key, child] of Object.entries(value)) {
    if (key === "path" && typeof child === "string") out.push(child);
    else collectPaths(child, out);
  }
}

function isInside(cwd: string, candidate: string): boolean {
  const rel = path.relative(cwd, candidate);
  return rel !== "" && !rel.startsWith("..") && !path.isAbsolute(rel);
}

function toPosix(rel: string): string {
  return rel.split(path.sep).join("/");
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
