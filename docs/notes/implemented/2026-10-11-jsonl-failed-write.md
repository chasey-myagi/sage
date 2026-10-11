# JSONL 追加失败时截掉半行

## Problem

`JsonlLogStore.persist` 循环处理短写，但 `writeSync` 中途抛错或返回 ≤0 时，已写出的前半行留在文件末尾。内存不提交这条 entry，可同一进程下一次 append 用的是同一个追加 fd，新行直接接在半行后面，拼成一行坏 JSON。这一行之后再有任何 append，重开就在中间行抛 `LogCorruptedError`；如果它恰好是最后一行，重开会把它当撕裂尾行丢掉，连带丢掉那条已经成功返回的 entry。文件不再是内存的超集。

## Decision

失败时截掉本次写出的字节；截断也失败时，拒绝该会话后续的写入。`persist` 在写行循环里累计这一行已确认写出的字节数 `written`；循环抛错（`writeSync` 抛错或返回 ≤0）时，若 `written > 0`，执行 `ftruncateSync(fd, fstatSync(fd).size - written)`，然后重新抛出原来的写错误，调用方拿到的仍是 `disk full` 或 `no progress` 这一类真实原因。fd 以追加模式打开，文件末尾的 `written` 个字节就是这一行写出的部分，即使另一个写者在这之前追加过整行，截断也碰不到它。成功路径不多任何系统调用。`ftruncateSync` 或 `fstatSync` 抛错时吞掉它，把会话标为不可写并存下这个错误；之后的 append 不碰文件，直接抛 `has a partial line that could not be removed`，`cause` 是存下的截断错误。半行因此一直是最后一行，新的 store 打开时按已有的撕裂尾行逻辑丢弃并截掉。`LogFs` 新增 `fstatSync`，生产实现用 `node:fs`。

## Alternatives considered

只拒绝后续写、不截断（失败一次就永久拒绝写）：实现最简单，但一次瞬时的 `EAGAIN` 或空间不足就让整个会话在本进程内报废，而截断回去只要两次系统调用。在内存里记录每个会话的文件长度，失败时截回写入前的长度：成功路径也要维护这份状态，打开时的长度必须算对，而且依赖单写者，另一个写者追加的整行会被一起截掉。每次写前 `fstatSync` 取写入前长度：成功路径每次 append 多一个系统调用，同样会截掉两次调用之间别的写者追加的内容；只在失败路径做一次 `fstatSync` 没有这两个问题。截断失败时抛一个包装后的新错误：会盖掉调用方真正需要的写失败原因，所以截断错误改在后续 append 的错误里作为 `cause` 交出。截断失败后在下次 append 时重试截断：多一条恢复路径，截断已经失败过的环境里很可能继续失败，收益不明，先不做。

## Consequences

store 多了一份不进 Log 的内存状态：不可写会话及其截断错误，文件头已注明。截断只依赖「追加 fd 写出的字节在文件末尾」，不依赖单写者，也不依赖打开时算出的长度；但如果另一个写者在本次失败的写和截断之间追加，截掉的会是对方的字节，这个窗口在单写者的前提下不存在，没有为它加锁。不可写的会话在本进程内只能换一个新的 `JsonlLogStore` 实例恢复。

## Testing

`packages/core/test/log/jsonl.test.ts` 新增四个用例：

- `removes the partial line when a write throws mid-line so the next append starts a clean line`
- `removes the partial line when a write makes no progress mid-line so the next append starts a clean line`
- `refuses later appends when the partial line of a failed write cannot be removed`
- `truncates only its own partial line when another writer appended after its last line`

前三个在修复前全红。前两个先写 5 字节再失败，接着成功 append 一条，重开断言 `torn` 为 false、两条 entry 的 seq 为 0 和 1，且与原 store 的 `all()` 一致。第三个让 `ftruncateSync` 抛错，断言后续 append 被拒绝且 `cause` 是截断错误、内存与 leaf 不变，重开时半行作为撕裂尾行被丢弃。第四个让另一个 store 实例在两次写之间追加一整行，断言本 store 写失败后对方的行仍在、文件没有撕裂。`pnpm check` 全绿。
