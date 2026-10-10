import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { run } from "../check-layers.mjs";

const layersScript = fileURLToPath(
  new URL("../check-layers.mjs", import.meta.url),
);
const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) {
    rmSync(root, { recursive: true, force: true });
  }
});

function makeRoot(): string {
  const root = mkdtempSync(join(tmpdir(), "sage-layers-"));
  roots.push(root);
  return root;
}

function write(root: string, rel: string, body: string): void {
  const file = join(root, rel);
  mkdirSync(join(file, ".."), { recursive: true });
  writeFileSync(file, body);
}

function writeLegal(root: string): void {
  write(
    root,
    "packages/provider/src/contract/types.ts",
    "export type Id = string;\n",
  );
  write(
    root,
    "packages/provider/src/contract/index.ts",
    'export type { Id } from "./types.ts";\nexport type ContractVersion = 1;\n',
  );
  write(
    root,
    "packages/provider/src/contract/dyn.ts",
    'export const loaded = import("./types.ts");\n',
  );
  write(
    root,
    "packages/provider/src/index.ts",
    'import "@sage/provider/contract";\nimport "node:fs";\nimport "left-pad";\nexport const PACKAGE = "@sage/provider" as const;\n',
  );
  write(
    root,
    "packages/core/src/index.ts",
    'import "@sage/provider/contract";\nexport const PACKAGE = "@sage/core" as const;\n',
  );
  write(
    root,
    "packages/plugins/src/index.ts",
    'import "@sage/core";\nexport const PACKAGE = "@sage/plugins" as const;\n',
  );
  write(root, "packages/tools/src/local.ts", "export const local = 1;\n");
  write(
    root,
    "packages/tools/src/index.ts",
    'import { local } from "./local.ts";\nexport const PACKAGE = "@sage/tools" as const;\nvoid local;\n',
  );
  write(
    root,
    "apps/sage/src/main.ts",
    'import "@sage/plugins";\nimport "@sage/tools";\nimport "../../../packages/core/src/index.ts";\nconsole.log("0.0.0");\n',
  );
}

function cli(root: string) {
  return spawnSync(process.execPath, [layersScript, root], {
    encoding: "utf8",
  });
}

describe("check-layers", () => {
  it("accepts a fixture that stays inside every layer rule", () => {
    const root = makeRoot();
    writeLegal(root);

    expect(run(root).violations).toEqual([]);

    const result = cli(root);
    expect(result.status).toBe(0);
    expect(result.stdout).toBe("");
  });

  it("rejects a non-relative import inside provider contract (rule 1)", () => {
    const root = makeRoot();
    write(
      root,
      "packages/provider/src/contract/leak.ts",
      'import "node:fs";\nimport "../index.ts";\n',
    );

    const { violations } = run(root);
    expect(violations).toEqual([
      {
        file: "packages/provider/src/contract/leak.ts",
        line: 1,
        rule: 1,
        source: "node:fs",
      },
      {
        file: "packages/provider/src/contract/leak.ts",
        line: 2,
        rule: 1,
        source: "../index.ts",
      },
    ]);

    const result = cli(root);
    expect(result.status).not.toBe(0);
    expect(result.stdout).toContain(
      "packages/provider/src/contract/leak.ts:1 违反规则 1：node:fs",
    );
    expect(result.stdout).toContain(
      "packages/provider/src/contract/leak.ts:2 违反规则 1：../index.ts",
    );
  });

  it("rejects provider imports of other @sage packages (rule 2)", () => {
    const root = makeRoot();
    write(
      root,
      "packages/provider/src/leak.ts",
      'import "@sage/core";\nexport { x } from "@sage/plugins";\nconst y = import("@sage/tools");\nvoid y;\n',
    );

    const { violations } = run(root);
    expect(
      violations.map((v) => ({ line: v.line, rule: v.rule, source: v.source })),
    ).toEqual([
      { line: 1, rule: 2, source: "@sage/core" },
      { line: 2, rule: 2, source: "@sage/plugins" },
      { line: 3, rule: 2, source: "@sage/tools" },
    ]);

    const result = cli(root);
    expect(result.status).not.toBe(0);
    expect(result.stdout).toContain(
      "packages/provider/src/leak.ts:1 违反规则 2：@sage/core",
    );
  });

  it("rejects core imports of plugins, tools, and apps (rule 3)", () => {
    const root = makeRoot();
    write(root, "packages/core/src/plugins.ts", 'import "@sage/plugins";\n');
    write(
      root,
      "packages/core/src/tools.ts",
      'export type { T } from "@sage/tools";\n',
    );
    write(
      root,
      "packages/core/src/apps.ts",
      'const app = import("../../../apps/sage/src/main.ts");\nvoid app;\n',
    );

    const { violations } = run(root);
    expect(violations).toEqual([
      {
        file: "packages/core/src/apps.ts",
        line: 1,
        rule: 3,
        source: "../../../apps/sage/src/main.ts",
      },
      {
        file: "packages/core/src/plugins.ts",
        line: 1,
        rule: 3,
        source: "@sage/plugins",
      },
      {
        file: "packages/core/src/tools.ts",
        line: 1,
        rule: 3,
        source: "@sage/tools",
      },
    ]);

    const result = cli(root);
    expect(result.status).not.toBe(0);
    expect(result.stdout).toContain(
      "packages/core/src/plugins.ts:1 违反规则 3：@sage/plugins",
    );
    expect(result.stdout).toContain("违反规则 3");
  });

  it("rejects plugin tool names spelled as string literals in core sources but not in comments or core tests (rule 3)", () => {
    const root = makeRoot();
    write(
      root,
      "packages/plugins/src/spawn.ts",
      'export const tool = { def: { name: "fan_out", description: "x" } };\nexport const NOTE_TOOL_NAME = "take_note";\nexport const scorer = { name: "rule", score() {} };\n',
    );
    write(
      root,
      "packages/core/src/health.ts",
      'export const isFan = (n: string) => n === "fan_out";\n// "fan_out" in a comment is fine\nexport const note = \'take_note\';\n/** The "take_note" tool. */\nexport type Kind = "rule" | "model";\nconst u = "https://x"; const n = "fan_out";\n',
    );
    write(
      root,
      "packages/core/test/health.test.ts",
      'export const name = "fan_out";\n',
    );

    const { violations } = run(root);
    expect(violations).toEqual([
      {
        file: "packages/core/src/health.ts",
        line: 1,
        rule: 3,
        source: '"fan_out" (plugin tool name)',
      },
      {
        file: "packages/core/src/health.ts",
        line: 3,
        rule: 3,
        source: '"take_note" (plugin tool name)',
      },
      {
        file: "packages/core/src/health.ts",
        line: 6,
        rule: 3,
        source: '"fan_out" (plugin tool name)',
      },
    ]);
  });

  it("rejects the provider root export from core sources but not from core tests (rule 3)", () => {
    const root = makeRoot();
    write(
      root,
      "packages/core/src/adapter.ts",
      'import { OpenAIAdapter } from "@sage/provider";\nvoid OpenAIAdapter;\n',
    );
    write(
      root,
      "packages/core/src/contract.ts",
      'export type { Message } from "@sage/provider/contract";\n',
    );
    write(
      root,
      "packages/core/test/adapter.test.ts",
      'import { OpenAIAdapter } from "@sage/provider";\nvoid OpenAIAdapter;\n',
    );

    expect(run(root).violations).toEqual([
      {
        file: "packages/core/src/adapter.ts",
        line: 1,
        rule: 3,
        source: "@sage/provider",
      },
    ]);
  });

  it("rejects plugins/tools cross-imports and apps imports (rule 4)", () => {
    const root = makeRoot();
    write(root, "packages/plugins/src/cross.ts", 'import "@sage/tools";\n');
    write(root, "packages/tools/src/cross.ts", 'import "@sage/plugins";\n');
    write(
      root,
      "packages/plugins/src/via-path.ts",
      'import "../../tools/src/cross.ts";\n',
    );
    write(
      root,
      "packages/tools/src/app.ts",
      'import "../../../apps/sage/src/main.ts";\n',
    );

    const { violations } = run(root);
    expect(violations).toEqual([
      {
        file: "packages/plugins/src/cross.ts",
        line: 1,
        rule: 4,
        source: "@sage/tools",
      },
      {
        file: "packages/plugins/src/via-path.ts",
        line: 1,
        rule: 4,
        source: "../../tools/src/cross.ts",
      },
      {
        file: "packages/tools/src/app.ts",
        line: 1,
        rule: 4,
        source: "../../../apps/sage/src/main.ts",
      },
      {
        file: "packages/tools/src/cross.ts",
        line: 1,
        rule: 4,
        source: "@sage/plugins",
      },
    ]);

    const result = cli(root);
    expect(result.status).not.toBe(0);
    expect(result.stdout).toContain(
      "packages/plugins/src/cross.ts:1 违反规则 4：@sage/tools",
    );
    expect(result.stdout).toContain("packages/tools/src/cross.ts");
    expect(result.stdout).toContain("违反规则 4");
  });
});
