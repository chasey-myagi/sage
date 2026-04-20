// ProviderSpec — Sprint 12 M1: Provider 强绑定表
//
// 设计文档：docs/design/model-id-policy.md
//
// 原则：
//   - Provider id 是代码级有限集，用户 YAML `llm.provider` 必须落在这里
//   - 每条 ProviderSpec 是 provider 的默认元数据（base_url / api_key_env /
//     api_kind / compat / 默认 max_tokens / 默认 context_window / 文档链接）
//   - 默认层级：YAML 覆盖 > ProviderSpec 默认 > 全局 fallback
//   - 元数据与 pi-mono `packages/ai/src/providers/*.ts` 保持一致（CLAUDE.md 强制要求）

use std::collections::HashMap;
use std::sync::LazyLock;

use super::types::{api, ProviderCompat, ThinkingFormat};

/// 单个 Provider 的静态元数据。
pub struct ProviderSpec {
    /// 稳定 id，YAML `llm.provider` 必须匹配这里的字符串
    pub id: &'static str,
    /// HTTP 默认 base_url。空串表示"运行时从环境变量解析"（Bedrock / Vertex / Azure）
    pub base_url: &'static str,
    /// API key 环境变量名
    pub api_key_env: &'static str,
    /// 该 provider 走哪套 HTTP wire format（决定 LlmProvider impl 的路由）
    pub api_kind: &'static str,
    /// 该 provider 默认的兼容性配置（YAML 无 override 时用这个）
    pub default_compat: ProviderCompat,
    /// 厂商 /v1/models 文档链接（错误提示引用）
    pub hint_docs_url: &'static str,
    /// 不填 `llm.max_tokens` 时的默认值（会被 YAML 覆盖）。
    ///
    /// **INVARIANT: must be non-zero** —— 验证由 `all_providers_have_non_zero_defaults`
    /// 测试强制。0 会让 `resolve_or_construct_model` 悄悄落到全局 `DEFAULT_MAX_TOKENS`
    /// fallback 上，prod 可能不崩但默认已经错位；写新 ProviderSpec 时保持非零。
    pub default_max_tokens: u32,
    /// 不填 `llm.context_window` 时的默认值（影响 compaction 阈值）。
    ///
    /// **INVARIANT: must be non-zero** —— 同 `default_max_tokens`。0 会让 compaction
    /// 失效或让 token budget 永为 0，用户 model 被强制截断。
    pub default_context_window: u32,
}

// ── ProviderCompat 工厂 ─────────────────────────────────────────────────────
//
// ProviderCompat 含 HashMap，不能 const，只能在 LazyLock 初始化时构造。

fn compat_default() -> ProviderCompat {
    ProviderCompat::default()
}

fn compat_zai() -> ProviderCompat {
    // pi-mono: { supportsDeveloperRole: false, thinkingFormat: "zai" }
    // zai API 不接受 developer role 消息；thinking 用 enable_thinking 字段
    ProviderCompat {
        supports_developer_role: false,
        thinking_format: Some(ThinkingFormat::Zai),
        ..ProviderCompat::default()
    }
}

// ── 全局 PROVIDERS 静态列表 ─────────────────────────────────────────────────

static PROVIDERS: LazyLock<Vec<ProviderSpec>> = LazyLock::new(|| {
    vec![
        // ── Anthropic 家族 ──
        ProviderSpec {
            id: "anthropic",
            base_url: "https://api.anthropic.com/v1",
            api_key_env: "ANTHROPIC_API_KEY",
            api_kind: api::ANTHROPIC_MESSAGES,
            default_compat: compat_default(),
            hint_docs_url: "https://docs.anthropic.com/en/docs/about-claude/models",
            default_max_tokens: 8192,
            default_context_window: 200_000,
        },
        // ── OpenAI 家族 ──
        ProviderSpec {
            id: "openai",
            base_url: "https://api.openai.com/v1",
            api_key_env: "OPENAI_API_KEY",
            api_kind: api::OPENAI_COMPLETIONS,
            default_compat: compat_default(),
            hint_docs_url: "https://platform.openai.com/docs/models",
            default_max_tokens: 16384,
            default_context_window: 128_000,
        },
        ProviderSpec {
            id: "google",
            base_url: "https://generativelanguage.googleapis.com/v1beta",
            // pi-mono env-api-keys.ts:115 — 不是 GOOGLE_API_KEY
            api_key_env: "GEMINI_API_KEY",
            api_kind: api::GOOGLE_GENERATIVE_AI,
            default_compat: compat_default(),
            hint_docs_url: "https://ai.google.dev/gemini-api/docs/models/gemini",
            default_max_tokens: 8192,
            default_context_window: 1_048_576,
        },
        ProviderSpec {
            id: "qwen",
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
            api_key_env: "DASHSCOPE_API_KEY",
            api_kind: api::OPENAI_COMPLETIONS,
            default_compat: compat_default(),
            hint_docs_url: "https://help.aliyun.com/zh/dashscope/developer-reference/model-list",
            default_max_tokens: 8192,
            default_context_window: 131_072,
        },
        ProviderSpec {
            id: "doubao",
            base_url: "https://ark.cn-beijing.volces.com/api/v3",
            api_key_env: "ARK_API_KEY",
            api_kind: api::OPENAI_COMPLETIONS,
            default_compat: compat_default(),
            hint_docs_url: "https://www.volcengine.com/docs/82379/1330310",
            default_max_tokens: 4096,
            default_context_window: 128_000,
        },
        ProviderSpec {
            id: "kimi",
            base_url: "https://api.moonshot.cn/v1",
            api_key_env: "MOONSHOT_API_KEY",
            api_kind: api::OPENAI_COMPLETIONS,
            default_compat: compat_default(),
            hint_docs_url: "https://platform.moonshot.cn/docs/api/models",
            default_max_tokens: 8192,
            default_context_window: 131_072,
        },
        ProviderSpec {
            id: "minimax",
            // pi-mono models.generated.ts:4557 — 走 anthropic 兼容路径
            base_url: "https://api.minimax.io/anthropic",
            api_key_env: "MINIMAX_API_KEY",
            api_kind: api::ANTHROPIC_MESSAGES,
            default_compat: compat_default(),
            hint_docs_url: "https://platform.minimaxi.com/document/guides/chat-model",
            default_max_tokens: 8192,
            default_context_window: 1_000_000,
        },
        ProviderSpec {
            id: "zai",
            // pi-mono models.generated.ts:13672 — 不是 open.bigmodel.cn
            base_url: "https://api.z.ai/api/coding/paas/v4",
            // pi-mono env-api-keys.ts:121 — 不是 ZHIPU_API_KEY
            api_key_env: "ZAI_API_KEY",
            api_kind: api::OPENAI_COMPLETIONS,
            // pi-mono: supportsDeveloperRole=false, thinkingFormat=zai
            default_compat: compat_zai(),
            hint_docs_url: "https://docs.z.ai/",
            default_max_tokens: 8192,
            default_context_window: 131_072,
        },
        ProviderSpec {
            id: "deepseek",
            base_url: "https://api.deepseek.com/v1",
            api_key_env: "DEEPSEEK_API_KEY",
            api_kind: api::OPENAI_COMPLETIONS,
            default_compat: compat_default(),
            hint_docs_url: "https://api-docs.deepseek.com/quick_start/pricing",
            default_max_tokens: 8192,
            default_context_window: 128_000,
        },
        ProviderSpec {
            id: "groq",
            base_url: "https://api.groq.com/openai/v1",
            api_key_env: "GROQ_API_KEY",
            api_kind: api::OPENAI_COMPLETIONS,
            default_compat: compat_default(),
            hint_docs_url: "https://console.groq.com/docs/models",
            default_max_tokens: 8192,
            default_context_window: 131_072,
        },
        ProviderSpec {
            id: "xai",
            base_url: "https://api.x.ai/v1",
            api_key_env: "XAI_API_KEY",
            api_kind: api::OPENAI_COMPLETIONS,
            default_compat: compat_default(),
            hint_docs_url: "https://docs.x.ai/docs/models",
            default_max_tokens: 8192,
            default_context_window: 131_072,
        },
        ProviderSpec {
            id: "cerebras",
            base_url: "https://api.cerebras.ai/v1",
            api_key_env: "CEREBRAS_API_KEY",
            api_kind: api::OPENAI_COMPLETIONS,
            default_compat: compat_default(),
            hint_docs_url: "https://inference-docs.cerebras.ai/introduction",
            default_max_tokens: 8192,
            default_context_window: 128_000,
        },
        ProviderSpec {
            id: "mistral",
            base_url: "https://api.mistral.ai/v1",
            api_key_env: "MISTRAL_API_KEY",
            api_kind: api::OPENAI_COMPLETIONS,
            default_compat: compat_default(),
            hint_docs_url: "https://docs.mistral.ai/getting-started/models/models_overview/",
            default_max_tokens: 8192,
            default_context_window: 128_000,
        },
        ProviderSpec {
            // github-copilot 代表性首个 model 走 anthropic-messages
            id: "github-copilot",
            base_url: "https://api.individual.githubcopilot.com",
            api_key_env: "COPILOT_GITHUB_TOKEN",
            api_kind: api::ANTHROPIC_MESSAGES,
            default_compat: compat_default(),
            hint_docs_url: "https://docs.github.com/en/copilot",
            default_max_tokens: 8192,
            default_context_window: 200_000,
        },
        // ── OpenRouter ── Linus v1 补齐（pi-mono env-api-keys.ts:119）
        ProviderSpec {
            id: "openrouter",
            base_url: "https://openrouter.ai/api/v1",
            api_key_env: "OPENROUTER_API_KEY",
            api_kind: api::OPENAI_COMPLETIONS,
            default_compat: compat_default(),
            hint_docs_url: "https://openrouter.ai/models",
            default_max_tokens: 8192,
            default_context_window: 200_000,
        },
        // ── 运行时解析 base_url 的三个（空串表示 implementer layer 解析）──
        ProviderSpec {
            id: "azure-openai-responses",
            base_url: "",
            api_key_env: "AZURE_OPENAI_API_KEY",
            api_kind: api::AZURE_OPENAI_RESPONSES,
            default_compat: compat_default(),
            hint_docs_url: "https://learn.microsoft.com/en-us/azure/ai-services/openai/concepts/models",
            default_max_tokens: 16384,
            default_context_window: 128_000,
        },
        ProviderSpec {
            id: "amazon-bedrock",
            base_url: "",
            api_key_env: "AWS_ACCESS_KEY_ID",
            api_kind: api::BEDROCK_CONVERSE_STREAM,
            default_compat: compat_default(),
            hint_docs_url: "https://docs.aws.amazon.com/bedrock/latest/userguide/models-supported.html",
            default_max_tokens: 8192,
            default_context_window: 200_000,
        },
        ProviderSpec {
            id: "google-vertex",
            base_url: "",
            api_key_env: "GOOGLE_CLOUD_API_KEY",
            api_kind: api::GOOGLE_VERTEX,
            default_compat: compat_default(),
            hint_docs_url: "https://cloud.google.com/vertex-ai/generative-ai/docs/learn/models",
            default_max_tokens: 8192,
            default_context_window: 1_000_000,
        },
    ]
});

/// 全部已知 Provider 的静态列表。
pub fn list_providers() -> &'static [ProviderSpec] {
    &PROVIDERS
}

static PROVIDER_MAP: LazyLock<HashMap<&'static str, &'static ProviderSpec>> =
    LazyLock::new(|| {
        PROVIDERS.iter().map(|spec| (spec.id, spec)).collect()
    });

/// 按 id 查找 Provider。大小写敏感；找不到返回 None。
pub fn resolve_provider(id: &str) -> Option<&'static ProviderSpec> {
    PROVIDER_MAP.get(id).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::api;

    // ── list_providers 基础行为 ───────────────────────────────────────────

    #[test]
    fn list_providers_contains_core_provider_subset() {
        // 锁死的核心子集：常用且稳定。补新 provider 不破坏这个测试。
        let specs = list_providers();
        let ids: Vec<&str> = specs.iter().map(|s| s.id).collect();
        for required in [
            "anthropic",
            "openai",
            "google",
            "kimi",
            "qwen",
            "deepseek",
            "openrouter",
        ] {
            assert!(
                ids.contains(&required),
                "list_providers() is missing core provider '{required}'"
            );
        }
    }

    #[test]
    fn list_providers_has_reasonable_size() {
        // 防退化的下限（至少 10 条），没有上限 — 补 provider 是常态不是回归。
        let n = list_providers().len();
        assert!(n >= 10, "expected ≥10 providers, got {n}");
    }

    #[test]
    fn list_providers_ids_are_unique() {
        let specs = list_providers();
        let mut seen = std::collections::HashSet::new();
        for spec in specs {
            assert!(
                seen.insert(spec.id),
                "duplicate provider id found: '{}'",
                spec.id
            );
        }
    }

    // ── resolve_provider 行为 ─────────────────────────────────────────────

    #[test]
    fn resolve_provider_known_id_returns_some() {
        assert!(
            resolve_provider("kimi").is_some(),
            "resolve_provider(\"kimi\") should return Some"
        );
    }

    #[test]
    fn resolve_provider_unknown_id_returns_none() {
        assert!(
            resolve_provider("not_a_real_one_xyz").is_none(),
            "resolve_provider with unknown id should return None"
        );
    }

    #[test]
    fn resolve_provider_is_case_sensitive() {
        assert!(
            resolve_provider("KIMI").is_none(),
            "resolve_provider(\"KIMI\") should be None — case sensitive"
        );
        assert!(
            resolve_provider("Kimi").is_none(),
            "resolve_provider(\"Kimi\") should be None — case sensitive"
        );
    }

    #[test]
    fn resolve_provider_empty_id_returns_none() {
        assert!(
            resolve_provider("").is_none(),
            "resolve_provider(\"\") should return None"
        );
    }

    #[test]
    fn resolve_provider_round_trip_for_all_listed_ids() {
        for spec in list_providers() {
            let found = resolve_provider(spec.id)
                .unwrap_or_else(|| panic!("missing '{}' in resolve_provider", spec.id));
            assert_eq!(found.id, spec.id, "round-trip id mismatch: {}", spec.id);
        }
    }

    // ── pi-mono drift-detection golden snapshot（#74 sub 4）─────────────
    //
    // 目的：锁死 v0.0.3 发布时 provider_specs.rs 的 (id, api_kind) 二元组
    // 全集。任何 provider 增删改都会让这个测试失败，提醒开发者**同时**去
    // pi-mono `packages/ai/src/providers/*.ts` 核对新旧 provider。
    //
    // 这不是等价性校验 —— pi-mono 与 Rust 侧的字段命名并不一一对应（TS
    // 用 camelCase，Rust 用 snake_case；compat 结构也不同）。它是一个
    // drift guard：失败时唯一正确的反应是 (a) 过一遍 pi-mono 看有没有
    // 新东西需要同步，(b) 更新下面的 EXPECTED 数组对齐当前状态。
    //
    // 测试失败的修复指引写在 panic 消息里，不要只改 EXPECTED 就完事。

    #[test]
    fn provider_specs_golden_snapshot_matches_expected() {
        // (id, api_kind) ordered by appearance in PROVIDERS vec. Keep in
        // lockstep with provider_specs.rs declarations above.
        const EXPECTED: &[(&str, &str)] = &[
            ("anthropic", api::ANTHROPIC_MESSAGES),
            ("openai", api::OPENAI_COMPLETIONS),
            ("google", api::GOOGLE_GENERATIVE_AI),
            ("qwen", api::OPENAI_COMPLETIONS),
            ("doubao", api::OPENAI_COMPLETIONS),
            ("kimi", api::OPENAI_COMPLETIONS),
            ("minimax", api::ANTHROPIC_MESSAGES),
            ("zai", api::OPENAI_COMPLETIONS),
            ("deepseek", api::OPENAI_COMPLETIONS),
            ("groq", api::OPENAI_COMPLETIONS),
            ("xai", api::OPENAI_COMPLETIONS),
            ("cerebras", api::OPENAI_COMPLETIONS),
            ("mistral", api::OPENAI_COMPLETIONS),
            ("github-copilot", api::ANTHROPIC_MESSAGES),
            ("openrouter", api::OPENAI_COMPLETIONS),
            ("azure-openai-responses", api::AZURE_OPENAI_RESPONSES),
            ("amazon-bedrock", api::BEDROCK_CONVERSE_STREAM),
            ("google-vertex", api::GOOGLE_VERTEX),
        ];
        let actual: Vec<(&str, &str)> =
            list_providers().iter().map(|s| (s.id, s.api_kind)).collect();
        if actual.as_slice() != EXPECTED {
            panic!(
                "\nProvider spec snapshot drift detected.\n\
                 Expected (v0.0.3 golden):\n  {:?}\n\
                 Actual:\n  {:?}\n\
                 \n\
                 Update steps when this test fails:\n  \
                 1. Cross-check ~/Dev/cc/external/pi-mono/packages/ai/src/providers/*.ts\n     \
                    for new/removed providers (CLAUDE.md alignment requirement).\n  \
                 2. Sync api_kind / default_compat / default_max_tokens here.\n  \
                 3. Update the EXPECTED array in this test to the new golden.\n",
                EXPECTED, actual,
            );
        }
    }

    // ── Kimi spec 具体字段验证 ─────────────────────────────────────────────

    #[test]
    fn kimi_spec_base_url_points_to_moonshot() {
        let spec = resolve_provider("kimi").unwrap();
        assert!(
            spec.base_url.starts_with("https://api.moonshot.cn"),
            "kimi base_url should start with 'https://api.moonshot.cn', got: {}",
            spec.base_url
        );
    }

    #[test]
    fn kimi_spec_api_key_env_is_moonshot_api_key() {
        let spec = resolve_provider("kimi").unwrap();
        assert_eq!(
            spec.api_key_env, "MOONSHOT_API_KEY",
            "kimi api_key_env should be 'MOONSHOT_API_KEY'"
        );
    }

    #[test]
    fn kimi_spec_api_kind_is_openai_completions() {
        let spec = resolve_provider("kimi").unwrap();
        assert_eq!(
            spec.api_kind,
            api::OPENAI_COMPLETIONS,
            "kimi api_kind should be openai-completions"
        );
    }

    #[test]
    fn kimi_spec_hint_docs_url_contains_moonshot_host() {
        let spec = resolve_provider("kimi").unwrap();
        assert!(
            spec.hint_docs_url.contains("moonshot"),
            "kimi hint_docs_url should contain 'moonshot', got: {}",
            spec.hint_docs_url
        );
    }

    // ── 各 Provider 的 api_kind 验证 ─────────────────────────────────────

    #[test]
    fn anthropic_spec_api_kind_is_anthropic_messages() {
        assert_eq!(
            resolve_provider("anthropic").unwrap().api_kind,
            api::ANTHROPIC_MESSAGES,
            "anthropic api_kind should be anthropic-messages"
        );
    }

    #[test]
    fn openai_spec_api_kind_is_openai_completions() {
        assert_eq!(
            resolve_provider("openai").unwrap().api_kind,
            api::OPENAI_COMPLETIONS,
            "openai api_kind should be openai-completions"
        );
    }

    #[test]
    fn google_spec_api_kind_is_google_generative_ai() {
        assert_eq!(
            resolve_provider("google").unwrap().api_kind,
            api::GOOGLE_GENERATIVE_AI,
            "google api_kind should be google-generative-ai"
        );
    }

    #[test]
    fn amazon_bedrock_spec_api_kind_is_bedrock_converse_stream() {
        assert_eq!(
            resolve_provider("amazon-bedrock").unwrap().api_kind,
            api::BEDROCK_CONVERSE_STREAM,
            "amazon-bedrock api_kind should be bedrock-converse-stream"
        );
    }

    // ── Important 补测（12 条 + pi-mono 对齐验证）─────────────────────────

    #[test]
    fn qwen_spec_api_kind_is_openai_completions() {
        assert_eq!(
            resolve_provider("qwen").unwrap().api_kind,
            api::OPENAI_COMPLETIONS,
        );
    }

    #[test]
    fn xai_spec_api_kind_is_openai_completions() {
        assert_eq!(
            resolve_provider("xai").unwrap().api_kind,
            api::OPENAI_COMPLETIONS,
        );
    }

    #[test]
    fn cerebras_spec_api_kind_is_openai_completions() {
        assert_eq!(
            resolve_provider("cerebras").unwrap().api_kind,
            api::OPENAI_COMPLETIONS,
        );
    }

    #[test]
    fn doubao_spec_api_kind_is_openai_completions() {
        assert_eq!(
            resolve_provider("doubao").unwrap().api_kind,
            api::OPENAI_COMPLETIONS,
        );
    }

    /// Linus v1 指控：pi-mono `models.generated.ts:4557` 明确写 minimax 走
    /// `https://api.minimax.io/anthropic` + anthropic API。之前走 OpenAI 是错。
    #[test]
    fn minimax_spec_api_kind_is_anthropic_messages_per_pi_mono() {
        let spec = resolve_provider("minimax").unwrap();
        assert_eq!(
            spec.api_kind,
            api::ANTHROPIC_MESSAGES,
            "minimax uses anthropic-messages per pi-mono models.generated.ts:4557"
        );
        assert!(
            spec.base_url.contains("minimax.io/anthropic"),
            "minimax base_url must include '/anthropic' suffix, got: {}",
            spec.base_url
        );
    }

    /// Linus v1 指控：pi-mono `env-api-keys.ts:121` 写 `ZAI_API_KEY`，
    /// `models.generated.ts:13672` 写 `api.z.ai/api/coding/paas/v4`，
    /// 并且 `supportsDeveloperRole=false`。之前一个都没对。
    #[test]
    fn zai_spec_matches_pi_mono() {
        let spec = resolve_provider("zai").unwrap();
        assert_eq!(
            spec.api_key_env, "ZAI_API_KEY",
            "pi-mono env-api-keys.ts:121 says ZAI_API_KEY"
        );
        assert!(
            spec.base_url.starts_with("https://api.z.ai"),
            "pi-mono says api.z.ai, got: {}",
            spec.base_url
        );
        assert!(
            !spec.default_compat.supports_developer_role,
            "zai does not accept 'developer' role messages"
        );
    }

    /// Linus v1 指控：pi-mono `env-api-keys.ts:115` 写 `GEMINI_API_KEY`，
    /// 不是 `GOOGLE_API_KEY`。
    #[test]
    fn google_spec_api_key_env_is_gemini_api_key_per_pi_mono() {
        assert_eq!(
            resolve_provider("google").unwrap().api_key_env,
            "GEMINI_API_KEY",
            "pi-mono env-api-keys.ts:115 says GEMINI_API_KEY"
        );
    }

    #[test]
    fn zai_spec_api_kind_is_openai_completions() {
        assert_eq!(
            resolve_provider("zai").unwrap().api_kind,
            api::OPENAI_COMPLETIONS,
        );
    }

    #[test]
    fn deepseek_spec_api_kind_is_openai_completions() {
        assert_eq!(
            resolve_provider("deepseek").unwrap().api_kind,
            api::OPENAI_COMPLETIONS,
        );
    }

    #[test]
    fn mistral_spec_api_kind_is_openai_completions() {
        assert_eq!(
            resolve_provider("mistral").unwrap().api_kind,
            api::OPENAI_COMPLETIONS,
        );
    }

    #[test]
    fn github_copilot_spec_api_kind_is_anthropic_messages() {
        assert_eq!(
            resolve_provider("github-copilot").unwrap().api_kind,
            api::ANTHROPIC_MESSAGES,
        );
    }

    #[test]
    fn azure_openai_responses_spec_api_kind_is_azure_openai_responses() {
        assert_eq!(
            resolve_provider("azure-openai-responses").unwrap().api_kind,
            api::AZURE_OPENAI_RESPONSES,
        );
    }

    #[test]
    fn google_vertex_spec_api_kind_is_google_vertex() {
        assert_eq!(
            resolve_provider("google-vertex").unwrap().api_kind,
            api::GOOGLE_VERTEX,
        );
    }

    #[test]
    fn groq_spec_api_kind_is_openai_completions() {
        assert_eq!(
            resolve_provider("groq").unwrap().api_kind,
            api::OPENAI_COMPLETIONS,
        );
    }

    #[test]
    fn openrouter_spec_matches_pi_mono() {
        // pi-mono env-api-keys.ts:119 — Sage 历史遗漏的第 18 个 provider
        let spec = resolve_provider("openrouter").unwrap();
        assert_eq!(spec.api_key_env, "OPENROUTER_API_KEY");
        assert!(spec.base_url.contains("openrouter.ai"));
        assert_eq!(spec.api_kind, api::OPENAI_COMPLETIONS);
    }

    // ── default_max_tokens / default_context_window ──────────────────────

    #[test]
    fn all_providers_have_non_zero_defaults() {
        // ProviderSpec 承担 provider-level 默认；0 会让下游 compaction / LLM 调用
        // 立刻崩，必须强制非零。
        for spec in list_providers() {
            assert!(
                spec.default_max_tokens > 0,
                "provider '{}' default_max_tokens is 0",
                spec.id
            );
            assert!(
                spec.default_context_window > 0,
                "provider '{}' default_context_window is 0",
                spec.id
            );
        }
    }

    #[test]
    fn anthropic_default_context_window_is_200k() {
        // Anthropic 全家默认 200K。Linus v1 抱怨一刀切 128K 让 Claude 用户
        // compaction 被迫早踢 35% — 这个测试锁死 per-provider 默认。
        assert_eq!(
            resolve_provider("anthropic")
                .unwrap()
                .default_context_window,
            200_000,
        );
    }

    #[test]
    fn google_default_context_window_is_gemini_scale() {
        // Gemini 1.5 Pro: 1M+ context。默认不能被 OpenAI 家的 128K 拖下来。
        assert!(
            resolve_provider("google").unwrap().default_context_window >= 1_000_000,
        );
    }
}
