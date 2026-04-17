# Model ID Policy — Provider 强绑 / Model ID 弱绑

## 背景与问题

当前 `llm/models.rs` 维护一张超大的 Model registry，每条记录把
`(provider_id, model_id)` 二元键绑到一组元数据（`base_url` / `api_key_env`
/ `context_window` / `max_tokens` / `cost` / `reasoning` / `compat` / …）上，
通过 `resolve_model(provider, model_id) -> Option<Model>` 查表。

**代价**：

1. **迭代成本高**。每新增一个可用模型 ID（如 `kimi-k2.5`、`qwen3.6-plus`、
   `claude-sonnet-4-7`），都要 commit + tests + review 一轮。厂商半个月
   出一个新模型，registry 永远在追。
2. **误导性元数据**。cost / context_window 由我们写死，但厂商定价和
   窗口随时调整，registry 里的数字**过时即误导**。
3. **黑箱错误**。用户 YAML 写错 model_id（如 `qwen3-plus` 而非
   `qwen3.6-plus`）时，`resolve_model` 返回 `None`，上层得到一个
   `UnknownModel` 错误但**不知道"正确的 ID 是什么"** —— 因为我们根本不
   掌握权威列表，厂商官方 API 才掌握。
4. **代码膨胀**。`models.rs` 已经 5500 行，80% 是模型定义样板，其中
   相当一部分模型用户**一辈子都不会调**。

## 设计原则

### Provider 是代码级有限集（强绑定）

Sage 代码里维护一个有限的 **Provider 枚举**（`anthropic` / `openai` /
`kimi` / `qwen` / `deepseek` / `bedrock` / `vertex` / `groq` / `xai` /
`ollama` / `openai_compat` / …）。每个 Provider 代码级注册以下元数据：

- `id: &'static str`
- `base_url: &'static str`（默认；YAML 可 override）
- `api_key_env: &'static str`
- `api_kind: ApiKind`（`openai_completions` / `openai_responses` /
  `anthropic` / `google_genai`）
- `default_compat: CompatConfig`（SSE 格式 / 字段名差异）
- 可选 `default_model_hint: &'static str`（仅用于"用户没填 model 时
  的兜底建议"日志，不强制）

YAML 的 `llm.provider` 字段必须落在这个有限集里 —— 否则
`validate_agent` / `load_config` 早在启动前就拒绝。

### Model ID 是字符串（弱绑定）

YAML 的 `llm.model` 字段是**任意字符串**，由用户负责填对。Sage 不查
registry、不 cross-check、不拒绝未知 ID。拿到字符串就**原样**塞进
Provider 的 HTTP 请求 `model` 字段。

相关字段同样**可选并从 YAML 读**：

```yaml
llm:
  provider: kimi          # 强验证
  model: kimi-k2.5        # 弱验证（只要求非空字符串）
  max_tokens: 8192        # 可选，不填用 Provider 默认 4096
  context_window: 262144  # 可选，不填不做 token 预算（compaction 行为降级）
  base_url: "https://..." # 可选 override，不填用 Provider 默认
```

`context_window` / `cost` 等过去 registry 里的"权威"元数据，现在
**全部从 YAML 读取**，不在代码里硬编码。用户对自己配的模型负责。

### Token / Cost 采集：强绑 Provider，弱绑 model

`TaskRecord` / `MetricsCollector` 里：

```rust
pub struct TaskRecord {
    // …
    pub provider: String,      // 强绑，必须落在有限集
    pub model: String,         // 弱绑，原样记录，不校验
    pub input_tokens: u64,     // 从 Provider SSE/response 里读，不算
    pub output_tokens: u64,
    // cost 字段去掉，或改成 Option（仅当 YAML 显式声明 cost_per_million 时算）
}
```

成本计算改成**后置可选**脚本：用户想算就写一份
`cost_schedule.yaml`（按 `(provider, model)` 键），离线对 `metrics/*.json`
批量补算。Sage 核心不负责计费数字的权威性。

### 错误提示：把黑盒错误还给用户

Provider 调用返回 4xx 且错误体里暗示模型名错误（Moonshot、DashScope、
Anthropic 都有类似 `"error": { "code": "invalid_model" }` 或
`"model_not_found"` 的约定）时，Sage 的 `llm::Error` 变体：

```rust
SageError::InvalidModel {
    provider: String,
    model_id: String,
    provider_error: String, // 原样转发
    hint: String,           // 静态提示：建议用户查 provider 的 /v1/models
}
```

日志里展示：

```
Provider 'kimi' rejected model 'kimi-k2' — "model_not_found".
Sage does not validate model IDs; check Moonshot's model list at
https://platform.moonshot.cn/docs/api/models or query:
  curl https://api.moonshot.cn/v1/models -H "Authorization: Bearer $MOONSHOT_API_KEY"
```

**hint 文本按 Provider 静态定制**（每个 Provider 知道自己的 models 端
点 URL 或文档链接）。

## TUI 交互

`sage init --agent <name>` 的 TUI 流程里：

### Provider 选择（下拉）

```
? Select LLM provider:
  ❯ kimi        (Moonshot — api.moonshot.cn, needs MOONSHOT_API_KEY)
    qwen        (DashScope — dashscope.aliyuncs.com, needs DASHSCOPE_API_KEY)
    anthropic   (Claude — api.anthropic.com, needs ANTHROPIC_API_KEY)
    openai      (OpenAI — api.openai.com, needs OPENAI_API_KEY)
    deepseek    ...
    …
```

从 `list_providers()` 代码级列表渲染。

### Model 输入（填空 + 历史补全）

```
? Model ID for 'kimi':
  (Enter to type, ↓ to choose from history)
  Previously used with kimi:
    kimi-k2.5
    moonshot-v1-auto
  > _
```

历史 model ID 的来源（聚合三路，去重）：

1. **metrics 扫描**：读 `~/.sage/agents/*/workspace/metrics/summary.json`
   里的 `(provider, model)` 对
2. **现有 agent configs 扫描**：读 `~/.sage/agents/*/config.yaml` 里
   `llm.model`
3. **Per-user known models cache**：`~/.sage/known_models.json`
   ```json
   {
     "kimi": ["kimi-k2.5", "moonshot-v1-auto"],
     "qwen": ["qwen3.6-plus", "qwen-plus"]
   }
   ```
   每次 Provider 首次调用成功（200 响应）后追加当前 model（去重），
   每次 `sage init` 时更新。

用户可以**输入任意新 ID**，Sage 不拒绝 —— 只要 Provider 端接受，就
自动进入后续的 known_models 缓存。输入错的，运行时 Provider 直接报
`InvalidModel` 错误（见上节）。

### 可选：在线 /v1/models 补全

各 Provider 的 OpenAI-compatible 端点几乎都支持 `GET /v1/models`。
TUI 里加一个
"Press 'p' to probe provider's live model list" 的 hint，调一次
`provider_models_probe(provider)` 拉真实列表作为补全源。这是
**加分项，不是 MVP**。

## 代码架构变化

### 新模块

- `llm/providers.rs` — `ProviderSpec` + `list_providers()` +
  `resolve_provider(id)`
- `llm/known_models.rs` — `~/.sage/known_models.json` 读写 + 聚合
  metrics / configs
- `llm/errors.rs`（或合并到已有）— `InvalidModel` + per-provider hint

### 收缩

- `llm/models.rs` 从 5500 行缩到**几百行**：
  - 保留 `ApiKind` / `CompatConfig` 等跨 Provider 类型
  - 删除所有具体 `Model {}` 字面量（迁移到 YAML 或 known_models）
  - 保留或迁移 `resolve_model` 测试（改成 `resolve_provider` +
    model ID passthrough）

### 迁移 TaskRecord

`TaskRecord.model` 已存在；加 `provider: String`；`cost` 字段判断
是否保留，保留则变 `Option<ModelCost>`（YAML 声明时才计算）。

## 迁移计划（M1-M3）

### M1 — Provider 提取 + 弱绑 model ID

1. 新增 `ProviderSpec` + `resolve_provider` 
2. `AgentConfig.llm` 解析改成：强验 provider，弱验 model
3. Provider 调用代码把 model 当字符串透传
4. `SageError::InvalidModel` 变体 + Provider 层 4xx 映射
5. `llm/models.rs` 里保留的是 per-provider **测试夹具**（不再是
   权威元数据）—— 改成小规模样本，标 `#[cfg(test)]` 或 `integration_samples`

### M2 — Metrics 解耦

1. `TaskRecord` 加 `provider: String`
2. `cost` 字段变 `Option<Cost>`，不强制
3. 离线 `cost_schedule.yaml` 补算脚本

### M3 — TUI 补全 + known_models cache

1. `~/.sage/known_models.json` 读写
2. `sage init` / `sage validate` TUI 交互
3. 可选：/v1/models 在线 probe（不进 v1.0，做 v1.0.1 ship 的 nice-to-have）

## 兼容性 / 影响面

- **configs/\*.yaml**：现有 `model: qwen3.6-plus` 之类的字符串**继续有效**
  ——从 resolve_model 查表路径切到透传路径，对 YAML 完全兼容
- **metrics/\*.json 历史文件**：只有新增 `provider` 字段，旧字段不动，
  serde `#[serde(default)]` 向后兼容
- **Cost 相关前端展示**（Runeforge / Usage 看板）：本来依赖 Sage 算的
  cost 不再由 Sage 给出；前端要么转成用 cost_schedule 算，要么只展示
  token 数
- **Bedrock / Vertex 多模型特例**：这两个 Provider 要求 Region + 特定
  ARN / URL，model_id 格式更复杂。ProviderSpec 保留 `url_template:
  Option<String>`，允许 `{model}` / `{region}` 占位符

## 开放问题

1. **Bedrock 的 inference profile ARN** 是 region 强相关的字符串，弱绑
   是否够？初步答案：够 —— 把完整 ARN 作为 model_id 直接透传，Bedrock
   Provider 不 parse，对方 endpoint 自己处理。
2. **Provider 默认 api_kind 与 base_url 的版本漂移**：Kimi / Qwen 有多
   个 `/v1` `/v3` 路径。`llm.base_url` YAML override 覆盖够用，但有没
   有必要引入 `api_version` 字段？— 倾向不引入，override 就够。
3. **老 `models.rs` 里声明的 "cost_per_million" 数字**：直接删？还是
   打包成 `configs/cost_schedule.example.yaml` 让用户参考？— 倾向后者，
   给个迁移锚点但不维护。
