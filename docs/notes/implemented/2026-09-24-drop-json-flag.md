# 删掉空操作的 `--json`

## Problem

`sage -p` 接受 `--json`，解析结果里有 `json: true`，但没有任何代码读它：stdout 本来就是那一行 `RunSummary` JSON。usage 与 README 把它列成一个旗标，读者会以为不写它就拿不到 JSON。

## Decision

按 G4「预发布不留兼容层」直接删除：选项表 `OPTIONS`、`sage -p` 的旗标列表（因而 usage）、`PrintOptions` 的 `json` 字段、README 的 usage 与说明。此后 `--json` 与任何未知旗标一样，打印 usage 并退出 1，stdout 为空。

## Alternatives considered

不保留为静默接受的别名。那样旗标仍是空操作，只是从文档里消失，等于留了兼容层。

不改成打印警告后继续运行。预发布阶段没有需要照顾的调用方，警告只会让两种写法长期并存。

## Consequences

仍在命令行里写 `--json` 的脚本会以退出码 1 失败，去掉这个参数即可，输出不变。

`apps/sage/test/e2e.test.ts` 的两个用例跟着改：「parses --json」钉的正是被删除的行为，换成「rejects --json like any unknown flag」；「prints health on sage -p --json and the same object from stats」测的是 health 输出，与 `--json` 无关，从 argv 里去掉 `--json` 并改名，断言不变。

## Testing

红灯时：`parseArgs(["--print", "hi", "--json"])` 返回 `{ ok: true, task: "hi", passthrough: false, json: true }`，与 `--nope` 的 usage 错误不相等。

删除后 `rejects --json like any unknown flag` 通过：`--print hi --json` 与 `-p --json hi` 的解析结果都等于 `--nope` 的结果；`runCli(["-p", "hi", "--json"])` 退出 1，stdout 为空，stderr 是不含 `--json` 的 usage。`pnpm check` 退出码 0（85 个文件、412 个测试）。
