# Log 追加不再逐行 fsync

## Problem

`JsonlLogStore.append` 每写一行都 `fsyncSync` 一次。GitHub runner 的磁盘慢，`persists 100 JSON lines…` 在 PR #61 的 CI 上超过 5 秒超时，本地同一测试只要 57ms。真实会话也要付这笔钱：每轮至少追加 manifest、request_header、assistant、tool result 几条，每条都等一次刷盘。

逐行 fsync 防的是整机断电和 OS 崩溃。sage 真正会遇到的是进程被杀：bench 超时、Ctrl-C、未捕获异常。`writeSync` 返回后，数据已经进了 OS 页缓存，进程死掉不影响它落到文件里。

## Decision

store 不再调用 fsync。`append` 仍然先 `writeSync` 一整行，写成功才进内存，文件永远是内存的超集，L1 的「先落盘再进内存」不变。截掉撕裂尾巴的 `ftruncateSync` 之后也不再 fsync。`LogFs` 去掉 `fsyncSync`。

保证改为：进程被杀不丢已追加的行；断电或 OS 崩溃可能丢尾部，重开时由已有的撕裂尾处理兜底，最后半行丢弃并截掉。写在 `jsonl.ts` 文件头和 `packages/core/README.md`「落盘保证」。

## Alternatives considered

同一个 tick 内的多次 append 合并成一次 write 加一次 fsync（#62 原提案）。不选。`append` 是同步的，调用方拿到返回值就认定已落盘。要合并，就得让 `append` 变成异步或引入写队列，整个内核的调用点都要跟着改，才能换回一个 sage 并不需要的断电保证。

加 `appendMany`。session 的几次追加分散在不同的 await 之间，没有一个自然的批次；只有 100 行测试会用上它。

`fdatasync` 替代 `fsync`。追加会改文件长度，fdatasync 仍要刷元数据，在慢盘上省不了多少。

保留 fsync，把测试超时调大。那是在测试里掩盖一个真实的性能开销，规范也不允许。

## Consequences

pi 和 kimi 的会话文件都是这个口径：追加即写，不逐行刷盘。以后要做断电级持久化（例如无人值守跑在别人机器上），应该在会话结束或阶段边界显式 flush 一次，而不是回到逐行 fsync。

`LogFs` 少了一个方法。仓库里只有 `jsonl.test.ts` 自己实现这个接口。

## Testing

红灯：`writes each entry once and never forces a disk sync`。用带计数的替身 fs 追加 100 条，断言 `writeSync` 100 次、`fsyncSync` 0 次，重开读回 100 条。改之前 `fsyncSync` 是 100 次。

`pnpm vitest run packages/core/test/log/jsonl.test.ts` 在本机从 2.24s 降到 0.2s；撕裂尾、中间行损坏、写失败不进内存几条既有用例不改，全部通过。`pnpm check` 全绿。
