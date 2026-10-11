# 发布形态：内部包出 dist，CLI 打成一个 npm 包

## Problem

所有包的 `exports` 都指向 `src/*.ts`。`apps/sage/bin/sage.mjs` 要带 `--experimental-transform-types` 重新 exec 自己，还要注册 `src/resolve-ts.mjs`，把 `./x.js` 回落到 `./x.ts`。Node 不给 `node_modules` 里的 `.ts` 擦类型，所以这个形状发不了 npm，用户机器上的 Node 也得支持这个实验旗标。

G4 规定首个 tag 之前只有一种形状，发布形态得在打 tag 之前定下来。

## Decision

- **内部包**：`@sage/provider`、`@sage/core`、`@sage/plugins`、`@sage/tools` 标 `private`，只在仓内使用。`exports` 每个出口写两个条件：`source` 指 `src/*.ts`，`default` 指 `dist/*.js`。`dist` 由 `tsc -b` 输出，原本就在 `.gitignore` 里。plugins、tools、apps 的 tsconfig 补上 references，因为下游包现在读的是上游的 `.d.ts`。
- **CLI 包**：`apps/sage` 改名 `@chasey-myagi/sage`，命令名仍是 `sage`。`build` 用 esbuild 把 `src/main.ts` 连同四个内部包打成 `dist/cli.js`，打包时带 `--conditions=source`，所以直接读源码，不依赖 tsc 的产物。内部包改为 devDependencies。`files` 只有 `bin` 和 `dist/cli.js`，tarball 没有运行时依赖。`prepack` 会先构建。
- **bin**：只做 `import { runCli } from "../dist/cli.js"`，然后用它的返回值退出。删掉了重新 exec、`resolve-ts.mjs`、`main.ts` 里直接运行时的分支，以及 app 包没有调用方的 `src/index.ts` 和 `exports`。
- **`sageBin()`**：源码里的 `src/bench/deps.ts` 和打包后的 `dist/cli.js` 目录深度不同，改成从当前模块往上找最近的 `package.json`，再拼上 `bin/sage.mjs`。没有改用 `process.argv[1]`：`runCli` 也会在进程内被调用（测试，以及以后嵌入的调用方），那时 `argv[1]` 是别的脚本，不是 sage 的 bin。
- **`engines.node >=22.18`**：和根 `package.json` 的 `engines` 一致（仓内脚本依赖 22.18 起默认开启的类型擦除），esbuild 的 `--target=node22.18` 也按这个下限打包，三处是同一个数。
- **测试**：`vitest.config.ts` 设 `ssr.resolve.conditions: ["source", "node"]`，测试不需要先构建。从 bin 起进程的用例和 `gen-log-manifest` 读构建产物，`pnpm check` 会先跑 `build`。
- **`gen-log-manifest.mjs`**：core 的 dist 现在能通过 `default` 条件解析到 provider 的 dist，所以删掉了它自带的 `.js` → `.ts` 解析钩子。
- **闸门**：`pnpm check` 改为 lint → `build`（`tsc -b` 加打包）→ … → test → `check:smoke`。smoke 依次执行 `pnpm pack`，断言 tarball 里的 `package.json` 没有 `dependencies` 字段，在空目录里用一个空的 npm cache（`--cache <临时目录>/npm-cache`）执行 `npm install --offline` 装 tarball，再跑 `sage --help`，检查 stderr 以 usage 开头。`--help` 按 app README 的约定退出 1，所以不看退出码。字段断言是确定性检查。空 cache 下的离线安装是第二道：只加 `--offline` 不换 cache，本机 npm cache 里有的第三方依赖照样能装上。CI 只跑 `pnpm check`，已经覆盖构建和 smoke。
- **pnpm**：`pnpm-workspace.yaml` 里 `allowBuilds: esbuild: false`。esbuild 的二进制随可选的平台包一起装好，它的 postinstall 只是再校验一遍，不需要运行。

## Alternatives considered

- **四个内部包各自发布，sage 按版本依赖它们**：否决，理由有三。
  - npm 上的 `@sage` scope 已经属于别人（Sage Group，例如 `@sage/design-tokens`），这条路要先给 scope 改名，波及每个源文件的 import、分层规则表和文档。
  - 会过早承诺 core、plugins、tools、provider 的库 API。现在没有外部调用方，设计文档也没写过要作为库发布。
  - 5 个包要同步版本，得引入 changesets 之类的发版流程。验收的 smoke 也得把 5 个 tarball 一起 pack、一起装。
  - 反过来，先发单个 CLI，以后要拆出库是加东西，不需要改已经发布的形状。
- **测试用 vitest alias 指向 src**：否决。它要维护一张和 `exports` 平行的路径表，加一个出口就要改两处。`source` 条件写在 `exports` 旁边，一个事实只有一个定义。
- **bin 直接跑 tsc 的 `dist/main.js`，内部包用 `bundledDependencies` 带进 tarball**：否决。tarball 里要放 4 个包的全部 dist 和一套 `node_modules` 目录，内部包的名字也还是会出现在已发布的依赖树里。和单文件 bundle 比，发布出去的东西更多，也换不来任何好处。
- **继续发 `.ts`，靠 Node 自带的类型擦除**：做不到。Node 拒绝给 `node_modules` 里的文件擦类型。

新增的构建依赖是 esbuild，只在 devDependencies 里。它删掉的自有代码有：`resolve-ts.mjs`（31 行）、bin 里的重新 exec 加解析钩子注册（约 15 行）、`gen-log-manifest.mjs` 里同样的解析钩子（约 15 行），以及 `main.ts` 直接运行时的分支（约 15 行），合计约 80 行。新写的自有代码是 `scripts/smoke-pack.mjs` 和 `sageBin` 往上找 `package.json` 的循环。

## Consequences

- 从仓库跑 CLI（`node apps/sage/bin/sage.mjs`、`sage bench run` 起的子进程）之前要先 `pnpm build`。改了源码不重新构建，跑的就是旧产物。
- `pnpm vitest run` 单跑 `apps/sage/test/e2e.test.ts` 里从 bin 起进程的用例，或者 `scripts/__tests__/gen-log-manifest.test.ts`，要先构建；其余用例直接读源码。
- 内部包的 `exports` 加出口时，`source` 和 `default` 两个条件都要写。
- 打包产物不带 sourcemap，报错堆栈里是 `dist/cli.js` 的行号。代码没有压缩，函数名还在。
- 包名 `@chasey-myagi/sage` 在首个 tag 之后就是对外的形状。

## Testing

- `pnpm check`：lint、build、分层、导出文档、README、Log 清单、93 个测试文件 499 个用例、smoke 全部通过。
- 给 tarball 临时加一个 `semver` 依赖，smoke 变红：报 `declares dependencies`。再临时关掉字段断言，只留空 cache，`npm install --offline` 报 `ENOTCACHED`，也是红。改动已恢复。
- `apps/sage/test/bench.test.ts` 钉住 `sageBin()` 从源码解析到 `apps/sage/bin/sage.mjs`。打包后的路径没有用例覆盖。它成立的依据是：装好的包里，`dist/cli.js` 往上最近的 `package.json` 就是包根，`bin/sage.mjs` 就在包根下。
- smoke 在 main（`464e8fe`）上是红的：把同一个脚本的包名换成 `sage` 跑，tarball 带着 `@sage/*@0.0.0` 依赖，`npm install --offline` 报 `ENOTCACHED`。
- 删掉所有 `dist/` 再跑 `pnpm vitest run`，只有上面提到的两个读构建产物的用例失败，其余都直接读源码通过。
- 手动按验收跑一遍：`pnpm pack` → 空目录 `npm i ./chasey-myagi-sage-0.0.0.tgz`，结果是 `added 1 package`；`npx sage --help` 打印 usage；不带 key 跑 `sage -p hi` 报缺 key、退出 1。全程没有实验旗标。
