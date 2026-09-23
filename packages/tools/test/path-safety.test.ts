import {
  mkdirSync,
  mkdtempSync,
  realpathSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { resolveInside } from "../src/path-safety.ts";

let dir = "";
const outsidePaths: string[] = [];

function freshDir(): string {
  dir = mkdtempSync(path.join(tmpdir(), "sage-path-"));
  return dir;
}

afterEach(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
  for (const target of outsidePaths.splice(0)) {
    rmSync(target, { recursive: true, force: true });
  }
});

function inside(cwd: string, input: string, abs: string) {
  expect(resolveInside(cwd, input)).toEqual({
    ok: true,
    path: realpathSync(abs),
  });
}

function outside(cwd: string, input: string) {
  expect(resolveInside(cwd, input)).toEqual({
    ok: false,
    result: {
      content: `[error] Path is outside the workspace: ${input}`,
      isError: true,
    },
  });
}

describe("resolveInside", () => {
  it("returns the real absolute path of a file inside the workspace", () => {
    const cwd = freshDir();
    const file = path.join(cwd, "a.txt");
    writeFileSync(file, "hi");
    inside(cwd, "a.txt", file);
    inside(cwd, path.join(cwd, "sub", "..", "a.txt"), file);
  });

  it("rejects a lexical escape before any read", () => {
    const cwd = freshDir();
    const escaped = path.join(path.dirname(cwd), "passwd-lookalike");
    outsidePaths.push(escaped);
    writeFileSync(escaped, "root:secret");
    outside(cwd, "../../etc/passwd");
    outside(cwd, escaped);
  });

  it("rejects a symlink whose target is outside the workspace", () => {
    const cwd = freshDir();
    const secret = path.join(path.dirname(cwd), "secret.txt");
    outsidePaths.push(secret);
    writeFileSync(secret, "secret");
    symlinkSync(secret, path.join(cwd, "link.txt"));
    outside(cwd, "link.txt");

    const outsideDir = path.join(path.dirname(cwd), "outside-dir");
    outsidePaths.push(outsideDir);
    mkdirSync(outsideDir);
    writeFileSync(path.join(outsideDir, "x.txt"), "secret");
    symlinkSync(outsideDir, path.join(cwd, "link-dir"));
    outside(cwd, path.join("link-dir", "x.txt"));
  });

  it("allows a symlink that stays inside the workspace", () => {
    const cwd = freshDir();
    const file = path.join(cwd, "a.txt");
    writeFileSync(file, "hi");
    symlinkSync(file, path.join(cwd, "inner.txt"));
    inside(cwd, "inner.txt", file);
  });
});
