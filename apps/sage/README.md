# sage

one-shot CLI。`node apps/sage/bin/sage.mjs -p "任务"`（或 `sage "任务"`）在当前目录跑完一轮，把 `RunSummary`（含 `health`）打成一行 JSON 写到 stdout。结果为 `done` 时退出 0，否则退出 1。

```
usage: sage [-p | --print] <task> [--provider <preset>] [--model <id>] [--base-url <url>] [--api-key-env <NAME>] [--ladder provider/model,provider/model] [--budget <tokens>] [--max-turns <n>] [--tool-network none|full] [--tool-sandbox none|full] [--passthrough] [--scorer rule|model|jev] [--shadow-scorer rule|model|jev] [--scorer-apply entry|rebuild]
       sage stats <session.jsonl>
       sage bench run --tasks <file> --policy passthrough|compiler --model <id> [--provider <preset>] [--budget <tokens>] [--max-turns <n>] [--tool-network none|full] [--tool-sandbox none|full] [--scorer rule|model|jev] [--shadow-scorer rule|model|jev] [--scorer-apply entry|rebuild] --out <dir>
       sage bench report <runA> <runB>
```

## 参数

任务写在 `-p` / `--print` 之后，或者 `--print=任务`；不写 `-p` 时，所有位置参数用空格连起来就是任务。两种同时出现，或者任务为空，打印 usage 并退出 1。旗标的值可以写成 `--flag value` 或 `--flag=value`；分开写时值不能以 `-` 开头。未知旗标、缺值、`--help` 都打印 usage 并退出 1。同一个旗标写两次，后写的生效。`sage -p` 与 `bench run` 共用一张参数表（`src/options.ts`）和同一套校验与报错文本。

## 会话

每次运行装一个 `AgentSession`：七个一方工具，加上 `update_ledger`、`expand`、`spawn_subtask`、`spawn_subtasks`。Log 写在 `<cwd>/.sage/sessions/<id>.jsonl`。子任务预设从 `<cwd>/.sage/subtasks` 和用户主目录读。视图是上下文编译器，带四个一方 claim 和「路由」一节的策略；`--passthrough` 改用 passthrough 视图。子任务总是编译。

`--budget <tokens>` 覆盖默认预算 40000，必须是正整数，否则退出 2，stderr 一行说明。和 `--passthrough` 一起用时忽略这个值，并在 stderr 提示一句。对照实验请显式压预算，默认 40000 在短任务上不会触发编译器。

`--max-turns <n>` 覆盖内核默认的 80 轮（`@sage/core` 的 `DEFAULT_MAX_TURNS`），必须是正整数，否则退出 2，stderr 一行说明。这个数交给 `session.run`，`--passthrough` 同样生效。

## 沙箱

`--tool-network none|full` 默认 `full`。`--tool-sandbox none|full` 是同一个开关，后写的覆盖先写的；值不合法时退出 2，报错里是用户写的那个旗标名。`none` 切断 bash 子进程的网络。在 darwin 上，`none` 还会拒绝读取同一临时根下的其他 `sage-bench-*` 工作区；当前工作目录仍可读，临时目录里的其他文件也可读。父进程照常请求模型。当前平台做不到网络切断时退出 2，stderr 一行说明，不会悄悄放开。linux 的 `unshare -n` 不能限制文件。

环境变量 `SAGE_SANDBOX_ALLOW_ROOT` 设了时，就是 bash 可读的根目录；没设时是当前目录的 realpath。它没有对应的 CLI 旗标，`bench run` 用它把本次运行根目录传给子进程。

## 打分

`--scorer rule|model|jev` 选根编译器的打分器，默认 `rule`（确定性的规则打分）。`model` 复用已装好的适配器和本次会话的型号。`--passthrough` 不编译上下文，两个打分旗标都不会调用打分器。

`--shadow-scorer rule|model|jev` 给根编译器装上影子打分，默认不装。它每轮只给新进入视图的工具块打分，结果写在清单的 `shadow` 上，`shadow.calls` 是这一轮真正发出的请求数（0 或 1）。影子只记不生效：不改档次、冻结和预算，也不改模型看到的消息。

`--scorer jev` 和 `--shadow-scorer jev` 把打分请求发到 `JEV_BASE_URL`（缺省 `https://api.typesafe.ai`）的 `/v1/systemone`。密钥读 `JEV_API_KEY`，没有则用 `TYPESAFE_API_KEY`。两个都没有时退出 2，stderr 写出这两个变量名。请求体里的 `model` 读 `JEV_MODEL`，缺省 `jev-latest`。接到本地 LLM2Jev（github.com/Yinsongxu/LLM2Jev）时，用它的 `--served-model-name` 作为 `JEV_MODEL`，例如 `JEV_BASE_URL=http://127.0.0.1:30000 JEV_API_KEY=local JEV_MODEL=那个名字 sage -p "任务" --scorer jev`。

`--scorer-apply entry|rebuild` 默认 `rebuild`：打分只在阶段重开时生效。`entry` 改为工具结果进入视图时就定档，前缀从第一次出现就是最终形态。DeepSeek 的缓存是前缀缓存，出窗口时再改档会把后面的近期内容每轮重新全价编码，按写入价和缓存读价要很多轮才回本。`entry` 必须同时写 `--scorer model` 或 `--scorer jev`，否则退出 2，stderr 点名 `--scorer model|jev`。规则打分不改档，入口生效没有意义。`entry` 不再另跑影子。`runHealth.entryScored` 汇总这次调用的次数、none / summary / full 和 usage。

## `sage stats`

`sage stats <session.jsonl>` 读一个会话文件，不启动运行，打印一行 JSON：`runHealth` 加上三个对账数，退出 0。`runHealth` 里的 `scorerCalls`、`entryScored.calls`、`shadow.calls` 都只数真正发出的打分请求，例如 `--shadow-scorer jev` 缺 key 的轮次不计。参数不是单个 `.jsonl` 路径时打印 usage 并退出 1；文件不存在或读不了时 stderr 一行说明并退出 1。

`shadowNone` 是影子判过 `none` 的块 id 个数，同一个 id 只算一次。`laterNeeded` 是其中后来仍需要的个数：该块 id 出现在更后面的 `expand` 参数 `id` 里，或者该块是 `read` 的结果，且同一 `path` 在更后的轮次又被 `read` 了一次。判 `none` 的那一轮里再次 read 不算后续轮次。`precision = 1 − laterNeeded / shadowNone`。`shadowNone` 为 0 时 `precision` 是 null。这三个数是事后标签，编译器不读它们。

## `sage bench`

离线跑分，不是 CI 闸门。

`bench run` 先查 key：写了 `--provider` 就要那个预设的环境变量，没写就要自动选择顺序里任一家的 key，报错文本和 `sage -p` 相同。缺 key 退出 2，此时还没读任务文件、没拉仓库。`--provider` 不是已知预设时打印 usage 并退出 1。

任务文件是 JSON 数组，每项 `{ id, repo, baseCommit, problem, testCommand, timeoutSec }`，可选 `setupCommand` 与 `testPatch`。任何一项不合法都在开跑前报错并退出 1。

每个任务依次执行，第一处失败决定结果：

1. 拉取到 `<realpath(tmpdir)>/sage-bench-<随机>/<taskId>/`：`git init`，加 origin，`git fetch --depth 1 --no-tags` 拉 `baseCommit`，`git checkout -q FETCH_HEAD`，再删掉 origin。`repo` 不带 scheme 时拉 `https://github.com/<repo>.git`。拉取失败是 `clone-failed`。拉完必须恰好一个提交、没有 tag、没有 remote，否则是 `clone-unclean`，setup 和 agent 都不跑。
2. 有 `setupCommand` 时在该目录用 `sh -c` 跑，受 `timeoutSec` 限制。非零退出是 `setup-failed`：不启动 sage，也不跑测试。
3. 在该目录跑 `sage -p <problem>`。argv 总带 `--max-turns`（写了旗标用那个数，没写就是 80）和 `--tool-network`；`--budget`、`--scorer`、`--shadow-scorer`、`--scorer-apply` 只在写了时转发，passthrough 下也转发。启动失败是 `sage-failed`，超时是 `timeout`。随后把会话 jsonl 复制到 `<out>/sessions/<taskId>.jsonl`。
4. `git status` 为空是 `empty-patch`（sage 自己的运行时目录 `.sage/` 不算改动，例如会话 Log 与溢出的工具输出），不跑测试；读不到 status 是 `diff-failed`。
5. 有 `testPatch` 时，先把 diff 头里每个路径从 `baseCommit` 恢复（`git checkout`，基线里没有的文件就删掉），再 `git apply`。失败是 `test-patch-failed`，不跑测试。agent 看不到 `testPatch`。
6. 跑 `testCommand`：退出 0 是 `resolved`，超时是 `timeout`，其他是 `tests-failed`。

`health` 是复制出来的会话的 `runHealth`，读不到时为 null。

`stoppedEarly` 是工具顺序的启发式指标：会话以 `done` 结束、最后一次成功的 `edit` / `write` 之后没有 `bash` 结果，且工作区 diff 非空时为 true。失败的编辑忽略，失败的 bash 也算一次检查尝试。不解析命令或输出，因此不能证明是否执行了测试，也不识别 bash 内的文件修改。空 diff 且会话可读时为 false；会话不可读、diff 读取失败或子进程超时未检查 diff 时为 null。它只影响统计，不改变 agent 行为或任务状态。

结果文件同时保存 `stoppedEarly`。报告的 `stoppedEarlyRate` 是 true 的任务数除以全部任务数（含未知项），空任务集为 null；差值仍为 B 减 A。

`<out>/results.json` 每跑完一个任务重写一次，只存 id、状态、原因、health 数字和 stoppedEarly，不复制题面。顶层字段：`budget` 是 compiler 策略下 `--budget` 的值，没写或 passthrough 时为 null；`toolNetwork` 默认 `none`，写 `--tool-network full` 才是 `full`；`maxTurns` 是 `--max-turns` 的值或 80，两种策略都记；`scorerApply` 是 `entry` 或 `rebuild`，没写记 `rebuild`。

bench 的网络默认 `none`，并总是写进子进程的 `--tool-network`。这时每个子进程的 bash 只能读本次运行根目录，任务可以 `cd` 到自己的绝对路径，其他 `sage-bench-*` 根目录仍被拒绝；运行根目录经 `SAGE_SANDBOX_ALLOW_ROOT` 传给子进程。开跑前，临时根下遗留的 `sage-bench-*` 目录会在 stderr 列出并提示手动清理，不会被删除。每个任务目录在任务结束时删除，运行根目录在整次运行结束时删除。全部任务跑完退出 0。

`bench report <runA> <runB>` 打印一个 JSON 对象 `{ a, b, delta }` 并退出 0。参数不是两个目录时打印 usage 并退出 1，缺 `results.json` 时退出 1。`a`、`b` 各带该次的 `budget`。比率都以任务数为分母。`tokensPerResolved` 是 `health.tokens.total` 之和除以 resolved 数，没有 resolved 时为 null。`cacheHitRatio` 和 `expandRate` 是各任务非 null 值的均值。`delta` 是 B 减 A，任一侧为 null 时该项为 null。

## Model Experience

### What the model sees

argv 和 stderr 进度不是模型可见面。本壳选定的 system prompt 和十一个工具 schema（七个一方工具，加上 `update_ledger`、`expand`、`spawn_subtask`、`spawn_subtasks`）会进入请求。两者都按 hash 写入 `request_header`：首见或与上一条不同时才存全文。进度行只在 stderr：`turn N · tokens in/out · tool calls`。

### Token effect

system prompt 是固定的几行；system prompt 与工具 schema 不变时，后续 `request_header` 只存两个 hash。工具页的长度由 `@sage/tools` 的双限位和 spill 卡住。

### KV cache effect

同一次运行里 system prompt 与工具 schema 不改，head 前缀保持稳定。工具结果接在前缀之后。

## 路由

默认梯子只有当前型号。多家 key 都在时，阶段重开不会改跑更便宜的型号。要跨型号，显式传 `--ladder provider/model,provider/model`，在这些档里选目录 input 价最低、且这次视图允许的那一档。新视图碰到 `.env` 或 `secrets` 时，非当前 provider 的档会被拿掉。`--help` 里有同一行说明。

## `--provider`

`--provider <name>` 选用预设。`--base-url` 覆盖该预设的基址，`--api-key-env` 覆盖读哪一个环境变量。没拿到 key 时退出 2，stderr 点名这个变量。不认识的预设退出 1。`--provider typesafe` 被拒绝并退出 1：目录里那一行是给 Jev 打分计价的，不是对话端点。没写 `--provider` 时，按 deepseek、dashscope、moonshot、openai、anthropic 取第一个已经设了 key 的预设；一个都没有时退出 1。显式 `--provider` 盖住这个顺序。不写 `--model` 时用该 provider 目录的第一条：deepseek 是 `deepseek-v4-pro`，`deepseek-flash` 用 `--model` 选。每个已经设了 key 的预设都会装上适配器：OpenAI 兼容的几家是同一个适配器类，基址不同。

| provider | base URL | 环境变量 | api |
| --- | --- | --- | --- |
| openai | `https://api.openai.com/v1` | `OPENAI_API_KEY` | openai-chat |
| deepseek | `https://api.deepseek.com` | `DEEPSEEK_API_KEY` | openai-chat |
| dashscope | `https://dashscope.aliyuncs.com/compatible-mode/v1` | `DASHSCOPE_API_KEY` | openai-chat |
| moonshot | `https://api.moonshot.cn/v1` | `MOONSHOT_API_KEY` | openai-chat |
| anthropic | `https://api.anthropic.com` | `ANTHROPIC_API_KEY` | anthropic-messages |
