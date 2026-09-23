# 传输层错误重试

## Problem

真机跑 deepseek-v4-pro passthrough，第 50 轮一次 `finish{kind:"error", error:{message:"fetch failed"}}`，会话 terminal `outcome: error`，前 49 轮作废。适配器约定传输失败以 finish error 结束、不抛，但流开始前的 fetch 抛错、429 和 5xx 都没有再试一次。

## Decision

重试放在 provider，包住响应体开始之前的那一次 POST。`fetch` 抛错（连接被拒、重置、DNS）、HTTP 429、HTTP 5xx 最多再试 2 次。第 n 次等待 `baseMs * 4^(n-1)`，缺省 `baseMs` 1000，所以是 1 秒然后 4 秒。构造参数 `retry: { attempts, baseMs }` 改这两个数。`sleep` 可注入，测试不真等。

其余 4xx 不重试。调用方 abort 不重试。响应体已经开始之后的断开、截断、解析失败不重试，照旧 finish error，已经交出的 delta 留着。

重试次数和最后一次失败原因写在这一轮的 usage 上：`details: { retries, reason }`。没有重试就不写。成功那次如果前面失败过，`reason` 是最后一次失败的消息。会话本来就把 usage 和 finish 抄进 assistant 条目的 `details`，Log 能看见，不新增条目种类。中间几次失败的响应体不进 Log。

## Alternatives considered

流已经开始后再重试。正文可能已经吐出一段，再 POST 一次会把同一段输出生成两遍，调用方无法把半截和第二次对上。所以响应体一开始就停，finish error 保留已经交出的文本。

在 core 里把整轮重跑。一轮里工具可能已经产生副作用，整轮重放会再跑一遍工具。这次的失败发生在模型请求内部，工具还没开始。放在适配器里只重发这一次 POST，不重开 turn。重试耗尽后 core 仍把这条 finish error 当成这一轮的结局。

用官方 SDK 自带的重试。SDK 的错误类型和重试策略会渗进本仓的词表，换一家 SDK 就得改上层。传输仍是自己的 `fetch`。

## Consequences

两个适配器的默认构造就会重试，CLI 不用改。一次 5xx 最多少等 5 秒（1+4）。中间失败的响应体不进 Log，只能看到次数和最后原因。`usage.details` 不是模型可见输入，下一轮请求体不带它。

已有「非 2xx 变成 finish error」的测试在 500 上注入了即时 `sleep`。断言没改。不注入的话，默认退避会超过单测超时。

## Testing

`pnpm check` 退出码 0（lint、tsc、分层、README、log manifest、vitest 365）。`packages/provider/test/regressions/issue-72.test.ts` 用注入的 fetch 和 sleep 覆盖两条适配器：fetch failed 一次后成功且 `retries === 1`、退避 1000；连续三次 503 后 finish error 且 `retries === 2`、退避 1000 与 4000；400 只请求一次；流中截断只请求一次并 finish error。不打真网。
