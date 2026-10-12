import { describe, expect, it } from "vitest";
import { classifyBashCommand } from "../../src/index.ts";

describe("classifyBashCommand", () => {
  it.each([
    "cat src/a.ts",
    "head -n 40 src/a.ts",
    "tail -20 log.txt",
    "sed -n '10,40p' src/a.ts",
    "grep -rn foo src",
    "rg 'a|b' src",
    "ls -la",
    "find . -name '*.ts'",
    "wc -l src/a.ts",
    "git diff HEAD~1",
    "git -C repo log --oneline -5",
    "git status",
  ])("treats %s as a reader", (command) => {
    expect(classifyBashCommand(command)).toBe("reader");
  });

  it.each([
    "pnpm test",
    "npx vitest run test/a.test.ts",
    "python3 scripts/run.py",
    "make build",
    "cargo test",
    "sed -i 's/a/b/' src/a.ts",
    "find . -name '*.tmp' -delete",
    "git commit -m 'wip'",
    "cat > src/new.ts <<'EOF'\nexport const a = 1;\nEOF",
    "echo hi > out.txt",
  ])("treats %s as a runner", (command) => {
    expect(classifyBashCommand(command)).toBe("runner");
  });

  it("drops a leading cd before judging the command", () => {
    expect(classifyBashCommand("cd /work/repo && cat a.ts")).toBe("reader");
    expect(classifyBashCommand("cd /work/repo && pnpm test")).toBe("runner");
  });

  it("collapses whitespace and newlines between parts", () => {
    expect(classifyBashCommand("  cat   a.ts \n\n  head  b.ts  ")).toBe(
      "reader",
    );
  });

  it("judges a pipeline by its first stage when later stages only filter", () => {
    expect(classifyBashCommand("pnpm test 2>&1 | tail -50")).toBe("runner");
    expect(classifyBashCommand("cat a.ts | grep -n export | head")).toBe(
      "reader",
    );
  });

  it("calls a pipeline mixed when a reader feeds a runner", () => {
    expect(classifyBashCommand("grep -l foo src | xargs rm")).toBe("mixed");
  });

  it("judges an && chain by every part", () => {
    expect(classifyBashCommand("cat a.ts && grep -n b b.ts")).toBe("reader");
    expect(classifyBashCommand("pnpm build && pnpm test")).toBe("runner");
    expect(classifyBashCommand("cat a.ts && pnpm test")).toBe("mixed");
    expect(classifyBashCommand("ls; pnpm test || echo failed")).toBe("mixed");
  });

  it("ignores echo and cd parts that only report or move", () => {
    expect(classifyBashCommand("pnpm test; echo $?")).toBe("runner");
    expect(classifyBashCommand("cd src")).toBe("reader");
  });

  it("does not split on operators inside quotes", () => {
    expect(classifyBashCommand(`grep -E "a && b|c; d" src`)).toBe("reader");
  });

  it("skips leading environment assignments", () => {
    expect(classifyBashCommand("LC_ALL=C sort a.txt")).toBe("reader");
    expect(classifyBashCommand("CI=1 pnpm test")).toBe("runner");
  });
});
