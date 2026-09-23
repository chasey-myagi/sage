# jevScorer 端点与密钥可配

## Problem

`jevScorer` 把请求发到写死的 `https://api.typesafe.ai`，CLI 只认 `TYPESAFE_API_KEY`。LLM2Jev 把本地模型包成兼容的 `POST /v1/systemone`，模型名由 `--served-model-name` 决定。基址、密钥和 model 这三处对不上，本地服务接不进去。

## Decision

`--scorer jev` 和 `--shadow-scorer jev` 读三个环境变量。`JEV_BASE_URL` 缺省 `https://api.typesafe.ai`。`JEV_API_KEY` 缺省回退 `TYPESAFE_API_KEY`。`JEV_MODEL` 缺省 `jev-latest`。两个 key 都没有时退出 2，stderr 写出这两个变量名。`jevScorer` 增加 `model` 选项，空字符串仍用 `jev-latest`，CLI 只把 `JEV_MODEL` 传进去。只设 `TYPESAFE_API_KEY` 时，URL、`Authorization` 和 `model` 与原来相同。

## Alternatives considered

为什么不加 `--jev-base-url`。聊天端点已经有 `--base-url`。打分服务和聊天服务不是同一个地址，再给打分器一条平行旗标，两套基址会缠在一起。环境变量把打分端点单独配出去。

为什么缺 key 仍点名 `TYPESAFE_API_KEY`。已有部署和测试只设了这个变量。新变量优先，旧变量是缺省回退。提示里两个名字都在，旧路径不用改配置。

为什么 `model` 放在 `jevScorer` 而不是由 CLI 改写请求体。请求体是打分器的契约。CLI 只透传名字。

## Consequences

没加依赖。子任务编译器仍不装这个打分器。`typesafe` 仍然不是聊天端点。测试注入 `fetchImpl`，不打真网。短程测试用 `--scorer-apply entry` 让打分器真正发出请求；默认的阶段重建路径没有改。

## Testing

`pnpm check` 退出码 0（biome、tsc、分层、README、log manifest，79 个文件、385 个测试）。覆盖自定义 `JEV_BASE_URL` / `JEV_API_KEY` / `JEV_MODEL` 打到假 fetch、影子打分同样透传、只有 `TYPESAFE_API_KEY` 时仍是 `https://api.typesafe.ai` 与 `jev-latest`、两个 key 都缺时退出 2 且 stderr 含两个变量名。
