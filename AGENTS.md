# 协作规矩

面向所有在这个仓库里干活的人和 agent runtime。设计文档在 [`docs/design/context-compiler.md`](./docs/design/context-compiler.md)，路线在 [issue #1](https://github.com/chasey-myagi/sage/issues/1)。

## 分层不变量

这三条是架构的地基，破了就没有「上下文编译器」这个卖点了。CI 用 grep 自检，不靠人读注释维持。

- **core 不认识 plugin。**内核只拥有 Log 与 View seam。块、档位、预算、申领单、打分、阶段——全部活在插件里。core 里出现任何 plugin 的名字或 import 都是错的。
- **plugin 之间不互相 import。**插件通过 claim 和编译器打交道，不通过彼此。要共享的东西下沉到 core 或独立包，不横向连线。
- **provider 的类型面是本仓自己的。**`Context` / `Message` / `Tool` / `Usage` 由 `packages/provider` 定义。可以用官方 SDK 做 HTTP 层，但换一个 SDK 不许波及上层——上层只见本仓的类型。

依赖方向总是从上往下：`apps` → `plugins` / `tools` → `core` → `provider`。core 只 import `@sage/provider/contract`（类型与纯函数），碰不到任何 HTTP 代码。下层不许反向依赖上层。规则表在 [`scripts/check-layers.mjs`](./scripts/check-layers.mjs)。

## 分支流

- feature 分支 → PR → `main`。不直接推 `main`。
- **PR 必须绿**才合：typecheck + test + 分层自检。
- 一个 PR 对应一件事。范围炸了就新开，不在原件上堆补丁。
- 不保留向后兼容：删掉过时路径，而不是加兼容层或迁移垫片。

## 测试规则

- **红灯先行。**先有一个会失败的测试钉住行为，再写实现。
- **不许放宽断言让测试变绿。**测试红了修根因，不改断言、不删用例、不加跳过标记。
- **断言面只有三个：`view` / `manifest` / `log`。**不许直接断言内部消息数组——换了原语那种测试就全废，这正是重写的原因之一。
- provider 一律用 fixture 替身，测试不打真网。
- 回归测试命名 `test/regressions/issue-<n>.test.ts`。
- `pnpm check` 是唯一的本地闸门，push 前必跑。
- **用例名描述行为**，不写 `scenario 8` 这类编号；读用例名就该知道钉住了什么。
- 测试替身只有一套：`@sage/core/testing`。新替身加在那里，不在各测试目录另起一份。

## 语言

- **说明用中文**：README、设计文档、PR 描述、issue、commit message。
- **代码与注释用英文**：标识符、类型名、代码注释、日志与错误信息。

## 契约红线

条款全文与出处见 [`docs/design/references.md`](./docs/design/references.md) §1，这里只列不许破的五条。

- **L5 · 三域边界**：可重放的事实进 Log（纯 JSON，无活对象）；活的拦截与瞬时信号是运行时事件；同一事实不许两个来源。
- **L4 · model-visible ⟺ logged**：任何进入模型请求的内容都必须能从 Log 重建；新增一种模型可见输入 = 新增一种 entry。请求信封（provider / model / 采样参数 / `systemPromptHash` / `toolsHash` / adapter 补的默认值）作为 `request_header` entry 落 Log，**内容寻址**：system prompt 与 tool schema 都按 hash 存，只有首见或与上一条 `request_header` 不同时才存全文。
- **G2 · Agent Note**：Problem / Decision / Alternatives considered / Consequences / Testing 五段（模板见 [`docs/notes/TEMPLATE.md`](./docs/notes/TEMPLATE.md)），非平凡 PR 必带；归档即冻结。
- **G4 · 预发布不留兼容层**：首个 tag 之前只有一种形状。
- **G7 · 文件头即契约**，含「故意不进 Log 的东西」。

## 代码规范

每条都能在评审里直接引用。理由与反例见 [`docs/development.md`](./docs/development.md)，踩过的坑见 [`docs/defensive-patterns.md`](./docs/defensive-patterns.md)。

**类型**

- **`strict` 下零 `any`。**外部输入——模型与工具 JSON、JSONL 行、HTTP 响应、argv、环境变量、文件内容——一律先是 `unknown`，经类型守卫再用。同进程里类型已经保证的值信任 TS，不重复做运行时校验。
- **`as` 只在两处合法**：守卫已经证明之后，和品牌类型的构造函数里。其他 `as` 附一句注释说明为什么收窄做不到。
- **多种情况用判别联合建模**（`type` / `kind` / `ok` 字段）。在封闭联合上 `switch` 以 `assertNever` 结尾，新增成员时编译器会指出每个漏掉的分支。
- **跨边界的不透明 id 用品牌类型**（`EntryId` 的写法），不传裸 `string`。
- **失败是正常结果时返回值，不抛异常。**工具返回 `isError`，provider 流以 `finish{kind:"error"}` 结束。`throw` 只留给程序缺陷和启动期配置错误。归一化失败时，分类字段（状态码、`transport` 之类）必须一路带到消费者，不许在中间层丢掉。

**结构**

- **一个函数做一件事。**超过 60 行或嵌套超过 3 层，是拆分信号；不拆就在函数上写明为什么。文件超过 600 行同理，入口壳也一样。
- **工厂函数加闭包优先**；`class` 只用于持有可变状态、生命周期长的对象（session、store、adapter）。
- **依赖从选项对象注入。**只为测试存在的字段标 `/** Test seam. <生产环境用什么>. */`。
- **可调参数是具名常量**（`UPPER_SNAKE`），一处定义、处处引用。同一个数字出现第二次就提取；跨包共享的默认值放在拥有它的那一层导出。
- **状态不编码进字符串。**`reason` 这类字符串只给人读和记 Log；逻辑判断读结构化字段，不用 `includes` / `startsWith`。
- **一个事实一个定义。**工具函数、常量、类型只定义一次，放在依赖图里所有调用方都够得到的最低一层（例如 `isRecord` 在 `@sage/provider/contract`，经 `@sage/core` 转出）。复制粘贴第二份之前先下沉。
- **平行的东西保持对称。**两个 adapter、两个子命令、两个打分器写法不对称时，要么提取公共部分，要么在代码里说明差异的原因。
- **优先用 Node 内置和维护良好的依赖，不手写基础设施。**参数解析用 `node:util` 的 `parseArgs`。新增运行时依赖要在 Agent Note 里写明它删掉了多少自有代码。
- **空 `catch` 写明吞掉了什么、为什么别处接不到**，`try` 只包一条语句。
- ESM；Node 内置模块带 `node:` 前缀；只作类型用的导入写 `import type`；相对导入写 `.js` 后缀。

**命名**

- **一个词一个意思。**同一个词在两个地方指不同的东西时，改掉其中一个（反例：`head` 同时指 system+tools、`messages[0]` 和一个 token 数）。
- 函数名以动词开头，布尔值用 `is` / `has` / `should` 开头；不用 `take`、`stay`、`fresh` 这类要读完实现才懂的名字。

## 注释与文档

写法细则见 [`sage-prose-standard`](./.agents/skills/sage-prose-standard/SKILL.md)。

- **文件头只写契约**：模块的职责、它对调用方的保证、故意不进 Log 的东西（G7）。不写逐个 PR 追加的行为变更；控制在 25 行以内，行为细节写到对应函数的 JSDoc 或包 README。
- **每个导出符号有 JSDoc**，写调用方需要、但签名表达不了的东西：返回值的区别、会抛什么、副作用、所有权、时序、取消。参数自明时不写 `@param`。`pnpm check:export-docs` 检查 JSDoc 是否存在，写得好不好靠评审。
- **函数体内的注释只写代码表达不了的**：不变量、竞态顺序、安全边界、出人意料的失败行为。不复述代码，不叙述控制流，不留评审过程。
- **领域术语在定义它的类型上解释一次**（chunk、tier、claim、epoch、stub），别处直接用。
- **不写裸 `(#12)`。**写一句结论；需要出处就链到 `docs/notes/` 的文件。
- **文档跟代码一起改。**配置、默认值、错误、Log 字段、CLI 旗标变了，同一个 PR 更新包 README 和 JSDoc。

## 交付

- 本地推送前的检查、commit 与 PR 的写法：[`sage-pre-push-checks`](./.agents/skills/sage-pre-push-checks/SKILL.md)。
- 评审清单：[`sage-code-review`](./.agents/skills/sage-code-review/SKILL.md)。
- 找可删、可合并的东西：[`sage-find-simplifications`](./.agents/skills/sage-find-simplifications/SKILL.md)。
- 项目 skill 的本体在 `.agents/skills/`，`.claude/skills` 是指向它的软链。改 skill 改本体。
