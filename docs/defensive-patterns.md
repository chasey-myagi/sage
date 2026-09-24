# 防御模式

每一条都是这个仓库真出过、或差点出的一类缺陷，写成防止它再犯的规则。写会话循环、provider、工具、沙箱、Log 持久化之前先读一遍。链接指向当时的 Agent Note。

## 失败交还给模型，不终止会话

工具的参数错误、路径越界、执行抛错，都写成 `isError: true` 的 `tool_result`，会话继续。只有内核自己的缺陷才终止。读越界曾经抛 `PathOutsideWorkspaceError`，异常从下一轮的 `accesses()` 冒到 `run`，整段会话变成 terminal error。工具 `execute` 与 `around` 的同步抛错、被拒绝的 Promise 都由内核包成 `tool_result`；工具自己能判断的失败直接返回 `fail(...)`，不抛。见 [工具错误不终止会话](./notes/implemented/2026-09-23-tool-error-no-kill.md)。

## 公开契约两侧都要守：归一化时保留分类

`ProviderAdapter.stream()` 不抛错，所有失败都以 `finish{kind:"error"}` 结束。把异常归一成 finish 的那一层，必须把消费者要用的分类（HTTP 状态码、是否传输错误、原始错误）一路带过去。曾经 `http.ts` 标了 `transport: true`，`drive.ts` 生成 finish 时只复制了 message 和 status，内核只好退回去匹配错误文案。新增一种分类，就把它写进 contract 的类型，而不是挂在对象上等下游去 `as` 出来。

## 重试按副作用分层

- 响应体开始之前的失败（fetch 抛错、429、5xx）由 provider 重试：1 秒、4 秒，次数记在 usage 上。见 [传输层重试](./notes/implemented/2026-09-23-provider-retry.md)。
- 流开始之后的传输断开由 core 整轮重发，前提是这一轮的 assistant 还没落 Log、工具还没执行：2 秒、8 秒，每次写 `turn_retry`。见 [整轮重试](./notes/implemented/2026-09-23-turn-retry.md)。
- 「传输失败」按连接是否在响应完成前结束来判，不按有没有抛错：读 body 抛错和 body 没收到结束信号就正常关闭是同一类，都打 `transport: true`。代理超时截断连接时两种表现都会出现。见 [未完成的流按传输失败重试](./notes/implemented/2026-09-24-incomplete-stream.md)。
- provider 不重开已经开始的流：半截 delta 已经交给调用方，再 POST 一次，两份输出对不上。
- 已经执行过的工具永远不重放。
- 4xx 不重试。

## 同一个 session 的 append 串行

`LogStore.append` 读当前 leaf 再写下一条，重叠调用会复用 `seq`、意外分叉链。同一个 sessionId 上的 append 必须串行；并行子任务各写各的 session。追加先写盘再进内存，磁盘永远是内存的超集。

## 做不到的隔离就失败，不悄悄放开

`--tool-network none` 在当前平台切不断网络时退出 2，而不是降级成联网运行。任何安全开关都一样：能力不满足就在启动时响亮地失败，并在错误信息里说明缺什么。bench 的可信度建立在这一条上。见 [工具断网](./notes/implemented/2026-09-23-tool-network.md)。

## 放行范围按最小一棵树算

沙箱先拒绝临时根下的兄弟工作区前缀，再放行本次运行根的 realpath。放行整个临时根会让 agent 读到另一次运行的结果。路径比较一律先 `realpath`：macOS 的 `/tmp` 是 `/private/tmp` 的软链。见 [放行本次运行根](./notes/implemented/2026-09-23-sandbox-allow-root.md)。

## 进入视图的字节不再改写

DeepSeek 等 provider 的缓存是前缀缓存。一个块进入视图之后改档，会让它后面的全部内容每轮按写入价重新编码，要几百轮才回本。档位在块第一次进入视图时定下并冻结，只在阶段边界整体重开。改编译器时，用前缀字节稳定性的测试兜住。见 [入口定档](./notes/implemented/2026-09-23-scorer-entry.md) 与 [阶段内累积冻结](./notes/implemented/2026-09-23-cumulative-freeze.md)。

## 解析模型输出：容忍包装，不猜内容

模型会在 JSON 前后加解释、套代码围栏。解析器去掉一层围栏，从第一个 `[` 或 `{` 按括号配对取一个值（字符串里的括号不计），前后文字忽略。取不出来就判失败并记下原文，不往后找第二个候选，也不按最后一个 `]` 截断。见 [打分器解析](./notes/implemented/2026-09-23-scorer-parse-extract.md)。

## 模型可见的东西都要能从 Log 重建

加一种进入请求的输入，就加一种 Log 条目（契约 L4）。旁路的内存状态、进程级缓存、没落盘的默认值，都会让离线回放与真实请求对不上。自检方法：只拿 Log 文件，能不能逐字节还原每一轮发给模型的请求。
