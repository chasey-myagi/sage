# 协作规矩

面向所有在这个仓库里干活的人和 agent runtime。设计文档在 [`docs/design/context-compiler.md`](./docs/design/context-compiler.md)，路线在 [issue #1](https://github.com/chasey-myagi/sage/issues/1)。

## 分层不变量

这三条是架构的地基，破了就没有「上下文编译器」这个卖点了。CI 用 grep 自检，不靠人读注释维持。

- **core 不认识 plugin。**内核只拥有 Log 与 View seam。块、档位、预算、申领单、打分、阶段——全部活在插件里。core 里出现任何 plugin 的名字或 import 都是错的。
- **plugin 之间不互相 import。**插件通过 claim 和编译器打交道，不通过彼此。要共享的东西下沉到 core 或独立包，不横向连线。
- **provider 的类型面是本仓自己的。**`Context` / `Message` / `Tool` / `Usage` 由 `packages/provider` 定义。可以用官方 SDK 做 HTTP 层，但换一个 SDK 不许波及上层——上层只见本仓的类型。

依赖方向总是从上往下：`apps` → `plugins` / `tools` → `provider` → `core`。下层不许反向依赖上层。

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

## 语言

- **说明用中文**：README、设计文档、PR 描述、issue、commit message。
- **代码与注释用英文**：标识符、类型名、代码注释、日志与错误信息。
