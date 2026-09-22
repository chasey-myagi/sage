import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { run } from "../check-readme-model-experience.mjs";

const readmeScript = fileURLToPath(
  new URL("../check-readme-model-experience.mjs", import.meta.url),
);
const roots: string[] = [];

const GOOD = `# pkg

## Model Experience

### Token effect

骨架阶段尚无模型可见面；本票不引入任何进入模型请求的内容。

### What the model sees

骨架阶段尚无模型可见面；本票不引入任何进入模型请求的内容。

### KV cache effect

骨架阶段尚无模型可见面；本票不引入任何进入模型请求的内容。
`;

afterEach(() => {
  for (const root of roots.splice(0)) {
    rmSync(root, { recursive: true, force: true });
  }
});

function makeRoot(): string {
  const root = mkdtempSync(join(tmpdir(), "sage-readme-"));
  roots.push(root);
  return root;
}

function write(root: string, rel: string, body: string): void {
  const file = join(root, rel);
  mkdirSync(join(file, ".."), { recursive: true });
  writeFileSync(file, body);
}

function exempt(
  root: string,
  entries: { package: string; reason: string }[],
): void {
  write(
    root,
    "scripts/readme-exemptions.json",
    `${JSON.stringify(entries, null, 2)}\n`,
  );
}

function cli(root: string) {
  return spawnSync(process.execPath, [readmeScript, root], {
    encoding: "utf8",
  });
}

describe("check-readme", () => {
  it("accepts READMEs that answer the three Model Experience questions", () => {
    const root = makeRoot();
    write(root, "packages/core/README.md", GOOD);
    write(root, "apps/sage/README.md", GOOD);
    exempt(root, []);

    expect(run(root).violations).toEqual([]);

    const result = cli(root);
    expect(result.status).toBe(0);
    expect(result.stdout).toBe("");
  });

  it("rejects a package README missing Model Experience when it is not exempt", () => {
    const root = makeRoot();
    write(root, "packages/core/README.md", "# core\n\n骨架。\n");
    exempt(root, []);

    const { violations } = run(root);
    expect(violations).toEqual([
      { package: "packages/core", message: "missing ## Model Experience" },
    ]);

    const result = cli(root);
    expect(result.status).not.toBe(0);
    expect(result.stdout).toContain("packages/core");
    expect(result.stdout).toContain("missing ## Model Experience");
  });

  it("rejects a Model Experience section that omits a required heading", () => {
    const root = makeRoot();
    write(
      root,
      "packages/plugins/README.md",
      "# plugins\n\n## Model Experience\n\n### What the model sees\n\nnone\n",
    );
    write(
      root,
      "apps/sage/README.md",
      [
        "# sage",
        "",
        "### What the model sees",
        "### Token effect",
        "### KV cache effect",
        "",
        "## Model Experience",
        "",
        "outside the section",
        "",
      ].join("\n"),
    );

    const { violations } = run(root);
    expect(violations).toEqual([
      { package: "apps/sage", message: "missing ### What the model sees" },
      { package: "apps/sage", message: "missing ### Token effect" },
      { package: "apps/sage", message: "missing ### KV cache effect" },
      { package: "packages/plugins", message: "missing ### Token effect" },
      { package: "packages/plugins", message: "missing ### KV cache effect" },
    ]);

    const result = cli(root);
    expect(result.status).not.toBe(0);
    expect(result.stdout).toContain("packages/plugins");
    expect(result.stdout).toContain("apps/sage");
  });

  it("accepts a missing section when the exemption lists that package with a reason", () => {
    const root = makeRoot();
    write(root, "apps/sage/README.md", "# sage\n\nCLI 壳。\n");
    exempt(root, [{ package: "apps/sage", reason: "CLI 壳，无模型可见面" }]);

    expect(run(root).violations).toEqual([]);

    const result = cli(root);
    expect(result.status).toBe(0);
  });

  it("rejects an exemption whose reason is empty", () => {
    const root = makeRoot();
    write(root, "apps/sage/README.md", "# sage\n\nCLI 壳。\n");
    exempt(root, [{ package: "apps/sage", reason: "" }]);

    const { violations } = run(root);
    expect(violations).toEqual([
      { package: "apps/sage", message: "exemption reason is empty" },
      { package: "apps/sage", message: "missing ## Model Experience" },
    ]);

    const result = cli(root);
    expect(result.status).not.toBe(0);
    expect(result.stdout).toContain("apps/sage");
    expect(result.stdout).toContain("exemption reason is empty");
  });
});
