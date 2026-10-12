# zx-1107 的 setup 在 npm 11 上能装上

## Problem

`ts-mini-2.json` 里 `google__zx-1107` 的 setup 是 `npm ci`。该提交的 `package-lock.json`（lockfileVersion 3）里，`@swc/core` 与 `esbuild` 的平台可选依赖只有 darwin-arm64 一条，其他平台的条目缺失。npm 10 的 `npm ci` 不管这些缺失，npm 11 的 `npm ci` 把它们算作锁文件与 `package.json` 不同步，以 `EUSAGE` 退出。Node 25 自带 npm 11，这条任务在那里永远是 `setup-failed`，模型没有机会出手，却仍计入 `errorRate` 的分母。zx 在该提交的 `engines` 是 `node >= 12.17.0`，没有限制 npm 版本。

## Decision

zx 的 setup 改成 `PUPPETEER_SKIP_DOWNLOAD=1 npm install --no-audit --no-fund --no-save && npm run build:js`，定义为 `fetch-multi-swe-ts.mjs` 里的具名常量 `NPM_LOCKED_NO_SAVE`，只给 zx 用，其余 `npm ci` 任务不动。

- 版本仍由锁文件决定：`npm install` 在锁文件条目满足 `package.json` 范围时直接沿用锁定版本。缺的只是本机平台的二进制包，它们的版本由父包 `optionalDependencies` 精确钉住（`@swc/core-*@1.10.15`、`@esbuild/*@0.25.0`），不经过范围解析。
- `--no-save` 不改写 `package-lock.json` 与 `package.json`，setup 之后 `git status` 仍然干净。
- 网络依赖与 `npm ci` 相同：从 registry 取锁定的 tarball，npm 缓存是温的就走缓存。不额外下载 npm 本身。

## Alternatives considered

`npx -y npm@10.9.3 ci …` 固定 npm 版本。它要先从 registry 取一份 npm，多一层网络依赖，跑的环境里 npm 版本和命令里的版本不再一致；而且 npm 10 的 `npm ci` 只装锁文件里有的条目，在非 darwin-arm64 的机器上装不到 esbuild 的平台二进制，`build:js` 起不来。这一点是从锁文件内容推出来的，没有在别的平台上实测。

`npm install --no-package-lock`（`NPM_NO_LOCK`，express 与 async-1790 在用）。它完全不读锁文件，按 `package.json` 的范围解析到当下最新的版本，测试结果会随时间漂移。express 和 async-1790 用它是因为没有可用的锁文件，zx 有。

## Consequences

如果以后 zx 这条的 `package.json` 与锁文件真的出现范围不一致，`npm install` 会悄悄重新解析那几项，而 `npm ci` 会报错。这个提交上没有这种情况：npm 11 装出的 504 个包与锁文件逐个版本相同，锁文件之外没有多出的条目。换别的基线提交时要重新做这一项比对。

其余仍用 `npm ci` 的 9 条，在 npm 11 下的 `npm ci --dry-run` 都能通过，不需要改；async-1790 的锁文件缺 watchify，npm 10 与 11 都过不了，它本来就用 `NPM_NO_LOCK`。用 corepack 的 8 条在 Node 25 上要求另装 corepack，README 的「准备环境」写明了。

## Testing

`pnpm check` 全绿，`fetch-multi-swe-ts --set ts-mini-2 matches ts-mini-2.json byte for byte from a fixture` 用新配置通过。真机验证（Node 25.9.0、npm 11.12.1，darwin-arm64）：原命令 `EUSAGE` 失败；新命令 setup 退出 0、`git status` 为空；打上隐藏测试后 `testCommand` 退出 1（110 条中 3 条失败），再打上数据集的 `fix_patch` 后退出 0（0 条失败）。命令全文在 PR 描述的「验证」里。
