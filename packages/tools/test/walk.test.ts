import {
  mkdirSync,
  mkdtempSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, expect, it } from "vitest";
import { searchWithoutRg } from "../src/grep.ts";
import { createTools } from "../src/index.ts";

const ctx = { sessionId: "s", toolUseId: "walk" };
let dir = "";

afterEach(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
  dir = "";
});

function workspace(): string {
  dir = mkdtempSync(path.join(tmpdir(), "sage-walk-"));
  const outside = path.join(dir, "outside");
  const cwd = path.join(dir, "ws");
  mkdirSync(outside);
  writeFileSync(path.join(outside, "linked.txt"), "needle\n");
  mkdirSync(path.join(cwd, "src", "deep"), { recursive: true });
  mkdirSync(path.join(cwd, "node_modules", "pkg"), { recursive: true });
  mkdirSync(path.join(cwd, ".git"));
  writeFileSync(path.join(cwd, "b.txt"), "needle\n");
  writeFileSync(path.join(cwd, "src", "a.txt"), "needle\n");
  writeFileSync(path.join(cwd, "src", "deep", "c.txt"), "needle\n");
  writeFileSync(path.join(cwd, "node_modules", "pkg", "x.txt"), "needle\n");
  writeFileSync(path.join(cwd, ".git", "HEAD.txt"), "needle\n");
  symlinkSync(path.join(outside, "linked.txt"), path.join(cwd, "link.txt"));
  symlinkSync(outside, path.join(cwd, "linkdir"));
  return cwd;
}

function findTool(cwd: string) {
  const found = createTools({ cwd }).find((item) => item.def.name === "find");
  if (!found) throw new Error("missing find");
  return found;
}

it("find and the in-process grep walk the same files: sorted, skipping node_modules, .git, and symlinks", async () => {
  const cwd = workspace();
  const expected = ["b.txt", "src/a.txt", "src/deep/c.txt"];

  const found = await findTool(cwd).execute({ glob: "**/*.txt" }, ctx);
  expect(found).toMatchObject({ isError: false, content: expected.join("\n") });

  const hits = await searchWithoutRg({
    cwd,
    pattern: "needle",
    root: cwd,
    maxResults: 200,
  });
  expect(hits).toMatchObject({
    isError: false,
    content: expected.map((file) => `${file}:1: needle`).join("\n"),
  });
});

it("find and the in-process grep list a file root as itself and a missing root as nothing", async () => {
  const cwd = workspace();

  const file = await findTool(cwd).execute(
    { glob: "**/*.txt", path: "src/a.txt" },
    ctx,
  );
  expect(file).toMatchObject({ isError: false, content: "src/a.txt" });
  const fileHits = await searchWithoutRg({
    cwd,
    pattern: "needle",
    root: "src/a.txt",
    maxResults: 200,
  });
  expect(fileHits).toMatchObject({
    isError: false,
    content: "src/a.txt:1: needle",
  });

  const missing = await findTool(cwd).execute(
    { glob: "**/*", path: "absent" },
    ctx,
  );
  expect(missing).toMatchObject({ isError: false, content: "No matches." });
});
