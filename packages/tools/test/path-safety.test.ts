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
import {
  PathOutsideWorkspaceError,
  resolveInside,
} from "../src/path-safety.ts";

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

describe("resolveInside", () => {
  it("returns the real absolute path of a file inside the workspace", () => {
    const cwd = freshDir();
    const file = path.join(cwd, "a.txt");
    writeFileSync(file, "hi");
    expect(resolveInside(cwd, "a.txt")).toBe(realpathSync(file));
    expect(resolveInside(cwd, path.join(cwd, "sub", "..", "a.txt"))).toBe(
      realpathSync(file),
    );
  });

  it("rejects a lexical escape before any read", () => {
    const cwd = freshDir();
    const outside = path.join(path.dirname(cwd), "passwd-lookalike");
    outsidePaths.push(outside);
    writeFileSync(outside, "root:secret");
    expect(() => resolveInside(cwd, "../../etc/passwd")).toThrow(
      PathOutsideWorkspaceError,
    );
    expect(() => resolveInside(cwd, outside)).toThrow(
      PathOutsideWorkspaceError,
    );
  });

  it("rejects a symlink whose target is outside the workspace", () => {
    const cwd = freshDir();
    const outside = path.join(path.dirname(cwd), "secret.txt");
    outsidePaths.push(outside);
    writeFileSync(outside, "secret");
    symlinkSync(outside, path.join(cwd, "link.txt"));
    expect(() => resolveInside(cwd, "link.txt")).toThrow(
      PathOutsideWorkspaceError,
    );

    const outsideDir = path.join(path.dirname(cwd), "outside-dir");
    outsidePaths.push(outsideDir);
    mkdirSync(outsideDir);
    writeFileSync(path.join(outsideDir, "x.txt"), "secret");
    symlinkSync(outsideDir, path.join(cwd, "link-dir"));
    expect(() => resolveInside(cwd, path.join("link-dir", "x.txt"))).toThrow(
      PathOutsideWorkspaceError,
    );
  });

  it("allows a symlink that stays inside the workspace", () => {
    const cwd = freshDir();
    const file = path.join(cwd, "a.txt");
    writeFileSync(file, "hi");
    symlinkSync(file, path.join(cwd, "inner.txt"));
    expect(resolveInside(cwd, "inner.txt")).toBe(realpathSync(file));
  });
});
