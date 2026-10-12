import { describe, expect, it } from "vitest";
import {
  bashFileOperands,
  bashSearchPatterns,
  classifyBashCommand,
  normalizeBashCommand,
} from "../../src/index.ts";

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
    "sort a.txt",
    "git diff HEAD~1",
    "git -C repo log --oneline -5",
    "git status",
    "git branch -a",
    "git tag",
    "git remote -v",
    "git worktree list",
    "git config --get user.name",
    "git stash list",
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
    "sort -o out.txt a.txt",
    "find . -name '*.tmp' -delete",
    "git commit -m 'wip'",
    "git branch -D old",
    "git stash pop",
    "cat > src/new.ts <<'EOF'\nexport const a = 1;\nEOF",
    "echo hi > out.txt",
    "echo hi>out.txt",
  ])("treats %s as a runner", (command) => {
    expect(classifyBashCommand(command)).toBe("runner");
  });

  it.each([
    "grep -rn 'x >= 3' src",
    "awk '$1 > 3' f",
    'rg "a > b"',
    "grep -n \\> src/a.ts",
  ])("does not read a quoted or escaped > in %s as a redirect", (command) => {
    expect(classifyBashCommand(command)).toBe("reader");
  });

  it("does not count stderr or descriptor redirects as file writes", () => {
    expect(classifyBashCommand("grep -rn foo src 2>/dev/null")).toBe("reader");
    expect(classifyBashCommand("cat a.ts 2>err.log")).toBe("reader");
    expect(classifyBashCommand("cat a.ts >&2")).toBe("reader");
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

  it("lets later reader stages filter a reader pipeline", () => {
    expect(classifyBashCommand("cat a.ts | grep -n export | head")).toBe(
      "reader",
    );
  });

  it("calls a runner piped onward mixed because its exit status is hidden", () => {
    expect(classifyBashCommand("pnpm test 2>&1 | tail -50")).toBe("mixed");
  });

  it("treats pipefail only as a set option that comes before the pipeline", () => {
    expect(classifyBashCommand("pnpm test 'pipefail' | tail -5")).toBe("mixed");
    expect(classifyBashCommand("pnpm test --grep pipefail | tail -5")).toBe(
      "mixed",
    );
    expect(classifyBashCommand("pnpm test | tail; set -o pipefail")).toBe(
      "mixed",
    );
    expect(classifyBashCommand("set -o pipefail && pnpm test | tail")).toBe(
      "runner",
    );
  });

  it("keeps a piped runner a runner when the command sets pipefail", () => {
    expect(
      classifyBashCommand("set -o pipefail; pnpm test 2>&1 | tail -50"),
    ).toBe("runner");
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

  it("judges a wrapper command by the command it runs", () => {
    expect(classifyBashCommand("sudo make install")).toBe("runner");
    expect(classifyBashCommand("env FOO=1 cat a.ts")).toBe("reader");
    expect(classifyBashCommand("time pnpm test")).toBe("runner");
    expect(classifyBashCommand("nice -n 5 cat a.ts")).toBe("reader");
    expect(classifyBashCommand("find . -name '*.ts' | xargs grep -n foo")).toBe(
      "reader",
    );
    expect(classifyBashCommand("command ls")).toBe("reader");
    expect(classifyBashCommand("exec pnpm test")).toBe("runner");
  });

  it("calls a loop or conditional mixed", () => {
    expect(classifyBashCommand("for f in *.ts; do cat $f; done")).toBe("mixed");
    expect(classifyBashCommand("if [ -f a ]; then cat a; fi")).toBe("mixed");
    expect(classifyBashCommand("while true; do sleep 1; done")).toBe("mixed");
    expect(classifyBashCommand("case $x in a) ls;; esac")).toBe("mixed");
  });

  it("drops comments outside quotes", () => {
    expect(classifyBashCommand("cat a.ts # then pnpm test")).toBe("reader");
    expect(classifyBashCommand("# just a note\npnpm test")).toBe("runner");
    expect(classifyBashCommand("grep -n '# pnpm test' a.ts")).toBe("reader");
  });
});

describe("normalizeBashCommand", () => {
  it.each([
    ["pnpm   test", "pnpm test"],
    ["pnpm test # again", "pnpm test"],
    ["cd repo && pnpm test", "cd repo&&pnpm test"],
    ["grep -rn 'a b' src", 'grep -rn "a b" src'],
    ["pnpm test;", "pnpm test"],
  ])("gives %j and %j the same key", (left, right) => {
    expect(normalizeBashCommand(left)).toBe(normalizeBashCommand(right));
  });

  it.each([
    ["pnpm test", "pnpm build"],
    ["echo a b", "echo 'a b'"],
    ["cat a | grep x", "cat a; grep x"],
    ["cat > f <<EOF\none\nEOF", "cat > f <<EOF\ntwo\nEOF"],
  ])("tells %j from %j", (left, right) => {
    expect(normalizeBashCommand(left)).not.toBe(normalizeBashCommand(right));
  });
});

describe("bashSearchPatterns", () => {
  it.each([
    ["grep -rn needle src", ["needle"]],
    ["rg -n -e first -e second .", ["first", "second"]],
    ["rg --regexp=inline .", ["inline"]],
    ["rg -A 3 -g '*.ts' 'two words' src", ["two words"]],
    ["cd repo && git log | grep -i fix", ["fix"]],
    ["xargs grep -l marker", ["marker"]],
    ["egrep -- -dash file", ["-dash"]],
    ["grep -E 'a|b' src", ["a|b"]],
    ["grep -r needle src", ["needle"]],
    ["rg -r NEW -E utf8 --color never --sort path needle .", ["needle"]],
    ["rg --replace NEW --encoding utf8 needle", ["needle"]],
    ["cat a.txt", []],
    ["grep -n", []],
  ])("reads the patterns of %j", (command, patterns) => {
    expect(bashSearchPatterns(command)).toEqual(patterns);
  });
});

describe("bashFileOperands", () => {
  it.each([
    ["cat src/a.ts", ["src/a.ts"]],
    ["cd /repo && cat src/a.ts", ["src/a.ts"]],
    ["sed -n '10,40p' src/a.ts", ["src/a.ts"]],
    ["sed -e 's/a/b/' -n src/a.ts", ["src/a.ts"]],
    ["head -n 40 src/a.ts src/b.ts", ["src/a.ts", "src/b.ts"]],
    ["awk -F, '{print $1}' data.csv", ["data.csv"]],
    ["wc -l a.ts", ["a.ts"]],
    ["cat a.ts | grep -n x", ["a.ts"]],
    ["sed -i 's/a/b/' f.ts", []],
    ["cat src/*.ts 2>/dev/null", []],
    ["pnpm test", []],
    ["grep -n x a.ts", []],
  ])("reads the files %j opens", (command, files) => {
    expect(bashFileOperands(command)).toEqual(files);
  });
});
