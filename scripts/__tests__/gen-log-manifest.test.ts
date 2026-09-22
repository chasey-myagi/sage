import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";

const script = fileURLToPath(
  new URL("../gen-log-manifest.mjs", import.meta.url),
);
const dirs: string[] = [];

afterEach(() => {
  for (const dir of dirs.splice(0))
    rmSync(dir, { recursive: true, force: true });
});

function run(args: string[]) {
  return spawnSync(process.execPath, [script, ...args], { encoding: "utf8" });
}

describe("gen-log-manifest", () => {
  it("exits 0 when the manifest matches and non-zero when it does not", () => {
    const dir = mkdtempSync(join(tmpdir(), "sage-manifest-"));
    dirs.push(dir);
    const file = join(dir, "log-manifest.md");

    const generated = run(["--file", file]);
    expect(generated.status, generated.stderr).toBe(0);

    const fresh = run(["--check", "--file", file]);
    expect(fresh.status, fresh.stderr).toBe(0);

    writeFileSync(file, `${readFileSync(file, "utf8")}stale\n`);
    const stale = run(["--check", "--file", file]);
    expect(stale.status).not.toBe(0);
    expect(stale.stderr).toContain("stale");
  });
});
