//! Per-user cache of `(provider, model_id)` pairs seen in successful Provider
//! calls (Sprint 12 M4 data layer).
//!
//! Wired into the chat loop by Sprint 12 task #72 — `record_session_model`
//! (see `serve.rs`) appends to the cache on every successful
//! `session.send()`. The `sage init` TUI dropdown consumer is still pending
//! (task #82), so a handful of read-path helpers remain unused for now;
//! they are individually `#[allow(dead_code)]` rather than muting the
//! whole module.

use std::collections::BTreeMap;
use std::path::Path;

/// Persisted per-user cache of model ids seen in successful Provider calls.
///
/// Stored as `~/.sage/known_models.json`. Keys are provider ids (must be in
/// `list_providers()`), values are sorted unique model id strings.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct KnownModels {
    #[serde(flatten)]
    pub by_provider: BTreeMap<String, Vec<String>>,
}

/// Load known models from a JSON file at `path`.
///
/// If the file does not exist, returns an empty [`KnownModels`] (not an error).
/// If the file exists but contains invalid JSON, returns `Err`.
pub fn load_known_models(path: &Path) -> std::io::Result<KnownModels> {
    match std::fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, e)
            })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok(KnownModels::default())
        }
        Err(e) => Err(e),
    }
}

/// Atomically write `models` to `path` as JSON.
///
/// Writes to a `.tmp` sibling first, then renames into place.
pub fn save_known_models(path: &Path, models: &KnownModels) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(models).map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, e)
    })?;

    // Build a unique tmp path: <original>.tmp.<pid>.<full-unix-nanos>
    //
    // Linus v1 fix: previously used `d.subsec_nanos()` which only keeps the
    // sub-second component (0..1e9) — two saves in different seconds could
    // collide on the nanos field alone. `as_nanos()` is the full monotonic
    // u128 unix-epoch nanosecond count, globally unique barring same-pid
    // same-instant calls (which `O_EXCL`-style rename already handles below).
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_name = format!(
        "{}.tmp.{}.{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("known_models.json"),
        pid,
        nanos,
    );
    let tmp = path.with_file_name(tmp_name);

    // Write to tmp; clean up on any failure.
    if let Err(e) = std::fs::write(&tmp, &json) {
        std::fs::remove_file(&tmp).ok();
        return Err(e);
    }

    // Rename tmp → final; clean up tmp on rename failure.
    if let Err(e) = std::fs::rename(&tmp, path) {
        std::fs::remove_file(&tmp).ok();
        return Err(e);
    }

    Ok(())
}

/// Load the cache at `path`, add `(provider, model)` if not already present,
/// then save. Creates the file if absent. Sorts models within each provider.
pub fn record_used_model(path: &Path, provider: &str, model: &str) -> std::io::Result<()> {
    let mut known = load_known_models(path)?;

    let models = known.by_provider.entry(provider.to_string()).or_default();
    if !models.contains(&model.to_string()) {
        models.push(model.to_string());
        models.sort();
    }

    save_known_models(path, &known)
}

/// Scan `agents_dir` for agent workspaces and aggregate all (provider, model)
/// pairs found in:
///   - `<agent>/config.yaml` under `llm.provider` / `llm.model`
///   - `<agent>/workspace/metrics/<ulid>.json` under `"provider"` / `"model"` keys
///
/// Returns a `BTreeMap<provider, sorted-unique-models>`.
/// Missing/malformed agents are skipped; missing `agents_dir` returns empty map.
///
/// Consumer: `sage init` TUI dropdown (task #82 pending). Tests exercise it.
#[allow(dead_code)]
pub fn aggregate_history_models(agents_dir: &Path) -> BTreeMap<String, Vec<String>> {
    let mut result: BTreeMap<String, Vec<String>> = BTreeMap::new();

    let entries = match std::fs::read_dir(agents_dir) {
        Ok(e) => e,
        Err(_) => return result,
    };

    for entry in entries.flatten() {
        let agent_dir = entry.path();
        if !agent_dir.is_dir() {
            continue;
        }

        // Read config.yaml → llm.provider + llm.model.
        //
        // SYNC: keep `MinLlm` / `MinConfig` fields in sync with
        // `sage_runner::config::AgentConfig.llm` (provider/model). If either
        // field is renamed or nested differently upstream, this shadow schema
        // silently returns empty aggregation — grep "SYNC:" at both sites when
        // touching AgentConfig.
        let config_path = agent_dir.join("config.yaml");
        if let Ok(yaml_bytes) = std::fs::read(&config_path) {
            #[derive(serde::Deserialize)]
            struct MinLlm {
                provider: Option<String>,
                model: Option<String>,
            }
            #[derive(serde::Deserialize)]
            struct MinConfig {
                llm: Option<MinLlm>,
            }
            match serde_yaml::from_slice::<MinConfig>(&yaml_bytes) {
                Ok(cfg) => {
                    if let Some(llm) = cfg.llm {
                        let prov = llm.provider.unwrap_or_default();
                        let mdl = llm.model.unwrap_or_default();
                        if !prov.is_empty() && !mdl.is_empty() {
                            result.entry(prov).or_default().push(mdl);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        agent = %agent_dir.display(),
                        error = %e,
                        "skipping malformed config.yaml in history aggregation",
                    );
                }
            }
        }

        // Read workspace/metrics/*.json (skip summary.json)
        let metrics_dir = agent_dir.join("workspace").join("metrics");
        if let Ok(metric_entries) = std::fs::read_dir(&metrics_dir) {
            for mentry in metric_entries.flatten() {
                let mpath = mentry.path();
                let fname = mpath
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                if fname == "summary.json" {
                    continue;
                }
                if mpath.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                if let Ok(bytes) = std::fs::read(&mpath) {
                    // SYNC: keep `provider` / `model` field names in sync
                    // with `sage_runner::metrics::TaskRecord`. Drift = silent
                    // empty aggregation. See note on MinConfig above.
                    #[derive(serde::Deserialize)]
                    struct MetricRecord {
                        #[serde(default)]
                        provider: String,
                        #[serde(default)]
                        model: String,
                    }
                    if let Ok(rec) = serde_json::from_slice::<MetricRecord>(&bytes) {
                        if !rec.provider.is_empty() && !rec.model.is_empty() {
                            result.entry(rec.provider).or_default().push(rec.model);
                        }
                    }
                }
            }
        }
    }

    // Dedup and sort all values
    for models in result.values_mut() {
        models.sort();
        models.dedup();
    }

    result
}

/// Return sorted, deduplicated model candidates for `provider` by merging:
///   1. `~/.sage/known_models.json` for that provider
///   2. `aggregate_history_models(agents_dir)` for that provider
///
/// Consumer: `sage init` TUI dropdown (task #82 pending).
#[allow(dead_code)]
pub fn candidates_for_provider(home: &Path, agents_dir: &Path, provider: &str) -> Vec<String> {
    let km_path = home.join(".sage").join("known_models.json");
    // Load failures fall back to an empty cache — candidates is a UX-layer
    // function (TUI suggestion list), fail-closed would leave the user with
    // no suggestions. Asymmetric with `record_used_model` which DOES propagate
    // Err on corrupt files (there the risk is silent data overwrite); here
    // the worst outcome is an empty dropdown.
    //
    // Linus v1 fix: Don't swallow silently — warn-log so a corrupt
    // `known_models.json` is visible to the operator.
    let known = match load_known_models(&km_path) {
        Ok(k) => k,
        Err(e) => {
            tracing::warn!(
                path = %km_path.display(),
                error = %e,
                "known_models.json unreadable; falling back to history-only candidates",
            );
            KnownModels::default()
        }
    };

    let history = aggregate_history_models(agents_dir);

    let mut candidates: Vec<String> = Vec::new();

    if let Some(models) = known.by_provider.get(provider) {
        candidates.extend(models.iter().cloned());
    }
    if let Some(models) = history.get(provider) {
        candidates.extend(models.iter().cloned());
    }

    candidates.sort();
    candidates.dedup();
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn make_known_models(pairs: &[(&str, &[&str])]) -> KnownModels {
        let mut m = KnownModels::default();
        for (provider, models) in pairs {
            m.by_provider.insert(
                provider.to_string(),
                models.iter().map(|s| s.to_string()).collect(),
            );
        }
        m
    }

    // ── load / save 基础 ──────────────────────────────────────────────────────

    /// 路径不存在 → Ok(KnownModels::default()), by_provider 为空 Map
    #[test]
    fn load_known_models_missing_file_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("known_models.json");
        let result = load_known_models(&path);
        assert!(result.is_ok(), "missing file must return Ok, got: {:?}", result.err());
        let km = result.unwrap();
        assert!(
            km.by_provider.is_empty(),
            "missing file must return empty map, got: {:?}",
            km.by_provider
        );
    }

    /// 路径存在但内容非法 JSON → Err（不是降级成空 map）
    #[test]
    fn load_known_models_returns_err_for_invalid_json() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("known_models.json");
        std::fs::write(&path, b"not valid json {{{{").unwrap();
        let result = load_known_models(&path);
        assert!(result.is_err(), "invalid JSON must return Err, not a default");
    }

    /// 写 {kimi: [k1, k2], openai: [g1]} → 读回完全一致
    #[test]
    fn save_then_load_preserves_structure() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("known_models.json");

        let km = make_known_models(&[
            ("kimi", &["k1", "k2"]),
            ("openai", &["g1"]),
        ]);
        save_known_models(&path, &km).unwrap();

        let loaded = load_known_models(&path).unwrap();
        assert_eq!(
            loaded.by_provider.get("kimi").map(|v| v.iter().map(|s| s.as_str()).collect::<Vec<_>>()),
            Some(vec!["k1", "k2"]),
            "kimi models must roundtrip"
        );
        assert_eq!(
            loaded.by_provider.get("openai").map(|v| v.iter().map(|s| s.as_str()).collect::<Vec<_>>()),
            Some(vec!["g1"]),
            "openai models must roundtrip"
        );
        assert_eq!(loaded.by_provider.len(), 2, "exactly 2 providers");
    }

    /// 写过程中 .tmp 文件存在，成功后只剩最终文件（happy path）
    #[test]
    fn save_is_atomic_tmp_then_rename() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("known_models.json");
        let km = make_known_models(&[("kimi", &["k1"])]);

        save_known_models(&path, &km).unwrap();

        // After a successful save, the final file must exist
        assert!(path.exists(), "final file must exist after save");

        // No lingering .tmp files
        let tmp_path = path.with_extension("json.tmp");
        let alt_tmp = {
            let mut p = path.clone();
            let fname = p.file_name().unwrap().to_string_lossy().to_string() + ".tmp";
            p.set_file_name(fname);
            p
        };
        assert!(
            !tmp_path.exists() && !alt_tmp.exists(),
            ".tmp file must not linger after successful save"
        );
    }

    /// save(默认 KnownModels) → 读回文件内容是空 object（flatten 语义）
    #[test]
    fn save_empty_known_models_produces_empty_object_json() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("known_models.json");

        save_known_models(&path, &KnownModels::default()).unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        // serde flatten on empty BTreeMap must produce `{}`
        let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert!(
            parsed.is_object(),
            "empty KnownModels must serialize as JSON object, got: {contents:?}"
        );
        // No unexpected keys — must be empty or only contain the by_provider wrapper
        // depending on flatten semantics. The important thing: reading it back gives empty.
        let loaded = load_known_models(&path).unwrap();
        assert!(
            loaded.by_provider.is_empty(),
            "roundtrip of empty KnownModels must still be empty"
        );
    }

    // ── record_used_model 去重 ────────────────────────────────────────────────

    /// 初次调用，路径不存在 → 文件创建，含 {provider: [model]}
    #[test]
    fn record_used_model_creates_file_when_absent() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("known_models.json");

        record_used_model(&path, "kimi", "kimi-k1").unwrap();

        assert!(path.exists(), "file must be created on first record");
        let loaded = load_known_models(&path).unwrap();
        assert_eq!(
            loaded.by_provider.get("kimi").map(|v| v.iter().map(|s| s.as_str()).collect::<Vec<_>>()),
            Some(vec!["kimi-k1"]),
        );
    }

    /// 已有 {kimi:[k1]}, record(kimi,k2) → {kimi:[k1,k2]}
    #[test]
    fn record_used_model_appends_new_model_to_existing_provider() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("known_models.json");

        let km = make_known_models(&[("kimi", &["k1"])]);
        save_known_models(&path, &km).unwrap();

        record_used_model(&path, "kimi", "k2").unwrap();

        let loaded = load_known_models(&path).unwrap();
        let models = loaded.by_provider.get("kimi").unwrap();
        assert!(models.contains(&"k1".to_string()), "k1 must still be present");
        assert!(models.contains(&"k2".to_string()), "k2 must be added");
        assert_eq!(models.len(), 2, "exactly 2 models");
    }

    /// 已有 {kimi:[k1]}, record(kimi,k1) → 仍是 {kimi:[k1]}（len=1）
    #[test]
    fn record_used_model_is_idempotent_on_duplicate() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("known_models.json");

        let km = make_known_models(&[("kimi", &["k1"])]);
        save_known_models(&path, &km).unwrap();

        record_used_model(&path, "kimi", "k1").unwrap();

        let loaded = load_known_models(&path).unwrap();
        let models = loaded.by_provider.get("kimi").unwrap();
        assert_eq!(models.len(), 1, "duplicate record must not increase len");
        assert_eq!(models[0], "k1");
    }

    /// {kimi:[k1]} + record(openai, gpt4) → 两个 provider keys
    #[test]
    fn record_used_model_opens_new_provider_bucket() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("known_models.json");

        let km = make_known_models(&[("kimi", &["k1"])]);
        save_known_models(&path, &km).unwrap();

        record_used_model(&path, "openai", "gpt-4").unwrap();

        let loaded = load_known_models(&path).unwrap();
        assert_eq!(loaded.by_provider.len(), 2, "must have 2 providers");
        assert!(loaded.by_provider.contains_key("kimi"), "kimi must remain");
        assert!(loaded.by_provider.contains_key("openai"), "openai must be added");
    }

    /// record(kimi, "kimi-k99-anything-xyz") → 存进去不校验（弱绑保持）
    #[test]
    fn record_used_model_accepts_any_model_string() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("known_models.json");

        let exotic = "kimi-k99-anything-xyz";
        record_used_model(&path, "kimi", exotic).unwrap();

        let loaded = load_known_models(&path).unwrap();
        let models = loaded.by_provider.get("kimi").unwrap();
        assert!(
            models.contains(&exotic.to_string()),
            "any model string must be accepted without validation"
        );
    }

    /// 连续 record(kimi, "z"), record(kimi, "a") → 列表按字母序排列
    #[test]
    fn record_used_model_sorts_models_within_provider() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("known_models.json");

        record_used_model(&path, "kimi", "z-model").unwrap();
        record_used_model(&path, "kimi", "a-model").unwrap();

        let loaded = load_known_models(&path).unwrap();
        let models = loaded.by_provider.get("kimi").unwrap();
        assert!(models.contains(&"a-model".to_string()), "a-model must be present");
        assert!(models.contains(&"z-model".to_string()), "z-model must be present");
        // Sorted: a-model < z-model
        let a_pos = models.iter().position(|m| m == "a-model").unwrap();
        let z_pos = models.iter().position(|m| m == "z-model").unwrap();
        assert!(a_pos < z_pos, "models must be sorted: a-model before z-model");
    }

    // ── aggregate_history_models ──────────────────────────────────────────────

    /// 准备两个 agent dir，每个有 config.yaml 含 llm.provider + llm.model
    #[test]
    fn aggregate_history_models_reads_config_yaml_llm_provider_and_model() {
        let tmp = TempDir::new().unwrap();
        let agents_dir = tmp.path();

        // agent1: kimi / kimi-k1
        let a1 = agents_dir.join("agent1");
        std::fs::create_dir_all(&a1).unwrap();
        std::fs::write(
            a1.join("config.yaml"),
            "name: agent1\ndescription: \"\"\nllm:\n  provider: kimi\n  model: kimi-k1\ngoal: test\ntools: {}\nconstraints:\n  max_turns: 5\n  timeout_secs: 60\n",
        )
        .unwrap();

        // agent2: openai / gpt-4
        let a2 = agents_dir.join("agent2");
        std::fs::create_dir_all(&a2).unwrap();
        std::fs::write(
            a2.join("config.yaml"),
            "name: agent2\ndescription: \"\"\nllm:\n  provider: openai\n  model: gpt-4\ngoal: test\ntools: {}\nconstraints:\n  max_turns: 5\n  timeout_secs: 60\n",
        )
        .unwrap();

        let result = aggregate_history_models(agents_dir);
        assert_eq!(
            result.get("kimi").map(|v| v.iter().map(|s| s.as_str()).collect::<Vec<_>>()),
            Some(vec!["kimi-k1"]),
            "kimi provider from config.yaml"
        );
        assert_eq!(
            result.get("openai").map(|v| v.iter().map(|s| s.as_str()).collect::<Vec<_>>()),
            Some(vec!["gpt-4"]),
            "openai provider from config.yaml"
        );
    }

    /// agent dir 下 workspace/metrics/<ulid>.json 含 provider/model 字段
    #[test]
    fn aggregate_history_models_reads_metrics_json_files() {
        let tmp = TempDir::new().unwrap();
        let agents_dir = tmp.path();

        let a1 = agents_dir.join("agent1");
        let metrics_dir = a1.join("workspace").join("metrics");
        std::fs::create_dir_all(&metrics_dir).unwrap();

        // Write a metrics file (no config.yaml — only metrics)
        std::fs::write(
            metrics_dir.join("01HV1234567890ABCDEFGHJKM.json"),
            r#"{"provider":"kimi","model":"kimi-k2.5","tokens":100}"#,
        )
        .unwrap();

        let result = aggregate_history_models(agents_dir);
        assert!(
            result.get("kimi").map(|v| v.contains(&"kimi-k2.5".to_string())).unwrap_or(false),
            "kimi-k2.5 must be extracted from metrics json"
        );
    }

    /// config.yaml 有 kimi/k1，metrics 也有 kimi/k1 → 聚合结果只出现一次
    #[test]
    fn aggregate_history_models_deduplicates_across_config_and_metrics() {
        let tmp = TempDir::new().unwrap();
        let agents_dir = tmp.path();

        let a1 = agents_dir.join("agent1");
        let metrics_dir = a1.join("workspace").join("metrics");
        std::fs::create_dir_all(&metrics_dir).unwrap();

        std::fs::write(
            a1.join("config.yaml"),
            "name: agent1\ndescription: \"\"\nllm:\n  provider: kimi\n  model: kimi-k1\ngoal: test\ntools: {}\nconstraints:\n  max_turns: 5\n  timeout_secs: 60\n",
        )
        .unwrap();
        std::fs::write(
            metrics_dir.join("01HV1234567890ABCDEFGHJKM.json"),
            r#"{"provider":"kimi","model":"kimi-k1"}"#,
        )
        .unwrap();

        let result = aggregate_history_models(agents_dir);
        let kimi_models = result.get("kimi").expect("kimi must be present");
        let count = kimi_models.iter().filter(|m| m.as_str() == "kimi-k1").count();
        assert_eq!(count, 1, "kimi-k1 must appear exactly once after dedup");
    }

    /// metrics json 里 provider="" (legacy) → 跳过不统计
    #[test]
    fn aggregate_history_models_skips_metrics_with_empty_provider() {
        let tmp = TempDir::new().unwrap();
        let agents_dir = tmp.path();

        let a1 = agents_dir.join("agent1");
        let metrics_dir = a1.join("workspace").join("metrics");
        std::fs::create_dir_all(&metrics_dir).unwrap();

        std::fs::write(
            metrics_dir.join("01HV1234567890ABCDEFGHJKM.json"),
            r#"{"provider":"","model":"some-model"}"#,
        )
        .unwrap();

        let result = aggregate_history_models(agents_dir);
        assert!(
            !result.contains_key(""),
            "empty provider key must not appear in aggregation"
        );
    }

    /// agents_dir 不存在 → 返回空 BTreeMap，不报错
    #[test]
    fn aggregate_history_models_returns_empty_for_missing_agents_dir() {
        let tmp = TempDir::new().unwrap();
        let agents_dir = tmp.path().join("nonexistent");

        let result = aggregate_history_models(&agents_dir);
        assert!(
            result.is_empty(),
            "missing agents_dir must return empty map, got: {:?}",
            result
        );
    }

    /// 某 agent 的 config.yaml 缺失或内容损坏 → 跳过，继续处理其他
    #[test]
    fn aggregate_history_models_skips_malformed_agents() {
        let tmp = TempDir::new().unwrap();
        let agents_dir = tmp.path();

        // good agent
        let good = agents_dir.join("good-agent");
        std::fs::create_dir_all(&good).unwrap();
        std::fs::write(
            good.join("config.yaml"),
            "name: good\ndescription: \"\"\nllm:\n  provider: kimi\n  model: kimi-k1\ngoal: test\ntools: {}\nconstraints:\n  max_turns: 5\n  timeout_secs: 60\n",
        )
        .unwrap();

        // bad agent: malformed config.yaml
        let bad = agents_dir.join("bad-agent");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("config.yaml"), b"{{{{not yaml").unwrap();

        let result = aggregate_history_models(agents_dir);
        // good-agent must still be processed
        assert!(
            result.get("kimi").map(|v| v.contains(&"kimi-k1".to_string())).unwrap_or(false),
            "good agent must still be aggregated even when another agent is malformed"
        );
    }

    // ── candidates_for_provider (合并层) ──────────────────────────────────────

    /// known_models has kimi=[k1]，aggregate has kimi=[k1,k2] → candidates = [k1, k2]
    #[test]
    fn candidates_for_provider_merges_known_models_and_history_dedup() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("home");
        let sage_dir = home.join(".sage");
        std::fs::create_dir_all(&sage_dir).unwrap();

        // known_models.json: kimi=[k1]
        let km_path = sage_dir.join("known_models.json");
        let km = make_known_models(&[("kimi", &["k1"])]);
        save_known_models(&km_path, &km).unwrap();

        // agents_dir with one agent that has kimi=[k1, k2]
        let agents_dir = tmp.path().join("agents");
        let a1 = agents_dir.join("agent1");
        std::fs::create_dir_all(&a1).unwrap();
        std::fs::write(
            a1.join("config.yaml"),
            "name: agent1\ndescription: \"\"\nllm:\n  provider: kimi\n  model: k2\ngoal: test\ntools: {}\nconstraints:\n  max_turns: 5\n  timeout_secs: 60\n",
        )
        .unwrap();

        let candidates = candidates_for_provider(&home, &agents_dir, "kimi");
        assert!(candidates.contains(&"k1".to_string()), "k1 from known_models must be present");
        assert!(candidates.contains(&"k2".to_string()), "k2 from history must be present");
        assert_eq!(candidates.len(), 2, "deduped: exactly 2 candidates");
    }

    /// 只有 kimi 数据，查 openai → 空 vec
    #[test]
    fn candidates_for_provider_empty_for_unknown_provider() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("home");
        let sage_dir = home.join(".sage");
        std::fs::create_dir_all(&sage_dir).unwrap();

        let km_path = sage_dir.join("known_models.json");
        let km = make_known_models(&[("kimi", &["k1"])]);
        save_known_models(&km_path, &km).unwrap();

        let agents_dir = tmp.path().join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();

        let candidates = candidates_for_provider(&home, &agents_dir, "openai");
        assert!(
            candidates.is_empty(),
            "no openai data → must return empty vec, got: {:?}",
            candidates
        );
    }

    /// 结果无重复、按字母序稳定输出
    #[test]
    fn candidates_for_provider_returns_sorted_unique() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("home");
        let sage_dir = home.join(".sage");
        std::fs::create_dir_all(&sage_dir).unwrap();

        // known_models: kimi=[z-model, a-model] (intentionally unsorted)
        let km_path = sage_dir.join("known_models.json");
        let km = make_known_models(&[("kimi", &["z-model", "a-model"])]);
        save_known_models(&km_path, &km).unwrap();

        // agents_dir with kimi=m-model (and duplicate a-model)
        let agents_dir = tmp.path().join("agents");
        let a1 = agents_dir.join("agent1");
        std::fs::create_dir_all(&a1).unwrap();
        std::fs::write(
            a1.join("config.yaml"),
            "name: agent1\ndescription: \"\"\nllm:\n  provider: kimi\n  model: m-model\ngoal: test\ntools: {}\nconstraints:\n  max_turns: 5\n  timeout_secs: 60\n",
        )
        .unwrap();
        let a2 = agents_dir.join("agent2");
        std::fs::create_dir_all(&a2).unwrap();
        std::fs::write(
            a2.join("config.yaml"),
            "name: agent2\ndescription: \"\"\nllm:\n  provider: kimi\n  model: a-model\ngoal: test\ntools: {}\nconstraints:\n  max_turns: 5\n  timeout_secs: 60\n",
        )
        .unwrap();

        let candidates = candidates_for_provider(&home, &agents_dir, "kimi");

        // No duplicates
        let mut dedup = candidates.clone();
        dedup.dedup();
        assert_eq!(candidates, dedup, "output must have no consecutive duplicates");
        let unique_count = {
            let mut s = candidates.clone();
            s.sort();
            s.dedup();
            s.len()
        };
        assert_eq!(candidates.len(), unique_count, "output must be fully deduplicated");

        // Sorted
        let mut sorted = candidates.clone();
        sorted.sort();
        assert_eq!(candidates, sorted, "output must be sorted alphabetically");
    }

    // ── test-review v1 Critical + Important 补测 ───────────────────────────

    #[test]
    fn save_known_models_cleans_up_tmp_on_write_failure() {
        let tmp = TempDir::new().unwrap();
        let target_dir = tmp.path().join("does").join("not").join("exist");
        let bad_path = target_dir.join("known_models.json");
        let km = make_known_models(&[("kimi", &["k1"])]);

        let result = save_known_models(&bad_path, &km);
        assert!(result.is_err(), "save to non-existent dir must fail");

        let mut ancestor = target_dir.as_path();
        while !ancestor.exists() {
            ancestor = match ancestor.parent() {
                Some(p) => p,
                None => break,
            };
        }
        if ancestor.is_dir() {
            for entry in std::fs::read_dir(ancestor).unwrap().flatten() {
                let name = entry.file_name();
                let s = name.to_string_lossy();
                assert!(
                    !s.ends_with(".tmp"),
                    "stale tmp file left behind: {}",
                    entry.path().display()
                );
            }
        }
    }

    #[test]
    fn candidates_for_provider_falls_back_to_history_when_known_models_missing() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();

        let agents_dir = tmp.path().join("agents");
        let a1 = agents_dir.join("agent1");
        std::fs::create_dir_all(&a1).unwrap();
        std::fs::write(
            a1.join("config.yaml"),
            "name: agent1\ndescription: \"\"\nllm:\n  provider: kimi\n  model: history-only-model\ngoal: t\ntools: {}\nconstraints:\n  max_turns: 5\n  timeout_secs: 60\n",
        )
        .unwrap();

        let candidates = candidates_for_provider(&home, &agents_dir, "kimi");
        assert!(
            candidates.contains(&"history-only-model".to_string()),
            "candidates must include history models even when known_models.json is missing, got: {candidates:?}"
        );
    }

    #[test]
    fn aggregate_history_models_returns_exact_provider_count() {
        let tmp = TempDir::new().unwrap();
        let agents_dir = tmp.path().join("agents");

        for (name, prov, model) in &[("a1", "kimi", "k1"), ("a2", "openai", "gpt-4")] {
            let dir = agents_dir.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("config.yaml"),
                format!("name: {name}\ndescription: \"\"\nllm:\n  provider: {prov}\n  model: {model}\ngoal: t\ntools: {{}}\nconstraints:\n  max_turns: 5\n  timeout_secs: 60\n"),
            ).unwrap();
        }

        let result = aggregate_history_models(&agents_dir);
        assert_eq!(
            result.len(),
            2,
            "exactly 2 provider keys expected, got {}: {:?}",
            result.len(),
            result.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn record_used_model_propagates_err_on_corrupt_existing_file() {
        let tmp = TempDir::new().unwrap();
        let km_path = tmp.path().join("known_models.json");
        std::fs::write(&km_path, "{not valid json").unwrap();

        let result = record_used_model(&km_path, "kimi", "kimi-k2");
        assert!(
            result.is_err(),
            "record_used_model on corrupt file must Err (got Ok — data would be silently overwritten)"
        );
    }

    #[test]
    fn aggregate_history_models_skips_metrics_with_empty_model() {
        let tmp = TempDir::new().unwrap();
        let agents_dir = tmp.path().join("agents");
        let a1 = agents_dir.join("a1");
        let metrics = a1.join("workspace").join("metrics");
        std::fs::create_dir_all(&metrics).unwrap();
        std::fs::write(
            metrics.join("01ABC.json"),
            r#"{"task_id":"01ABC","agent_name":"a1","provider":"kimi","model":"","config_hash":"","started_at":0,"ended_at":0,"duration_ms":0,"input_tokens":0,"output_tokens":0,"cache_read_tokens":0,"cache_write_tokens":0,"turn_count":0,"tool_call_count":0,"tool_error_count":0,"compaction_count":0,"success":true,"failure_reason":null,"session_type":"user_driven","crafts_active":[]}"#,
        ).unwrap();

        let result = aggregate_history_models(&agents_dir);
        if let Some(models) = result.get("kimi") {
            assert!(
                !models.iter().any(|m| m.is_empty()),
                "empty model must not appear in aggregated candidates: {models:?}"
            );
        }
    }

    #[test]
    fn candidates_for_provider_dedups_same_model_in_known_and_history() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let km_path = home.join(".sage").join("known_models.json");
        std::fs::create_dir_all(km_path.parent().unwrap()).unwrap();
        let km = make_known_models(&[("kimi", &["shared-k"])]);
        save_known_models(&km_path, &km).unwrap();

        let agents_dir = tmp.path().join("agents");
        let a1 = agents_dir.join("a1");
        std::fs::create_dir_all(&a1).unwrap();
        std::fs::write(
            a1.join("config.yaml"),
            "name: a1\ndescription: \"\"\nllm:\n  provider: kimi\n  model: shared-k\ngoal: t\ntools: {}\nconstraints:\n  max_turns: 5\n  timeout_secs: 60\n",
        ).unwrap();

        let candidates = candidates_for_provider(&home, &agents_dir, "kimi");
        let shared_count = candidates.iter().filter(|m| *m == "shared-k").count();
        assert_eq!(
            shared_count, 1,
            "same (provider, model) pair from known_models and history must dedup to 1 entry, got {shared_count} in {candidates:?}"
        );
    }

    /// Linus v1 blocker #2: corrupt known_models.json must fall back to
    /// history (not propagate Err, not panic), symmetric with the
    /// missing-file test above.
    #[test]
    fn candidates_for_provider_falls_back_on_corrupt_known_models() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("home");
        let km_path = home.join(".sage").join("known_models.json");
        std::fs::create_dir_all(km_path.parent().unwrap()).unwrap();
        std::fs::write(&km_path, "{not valid json at all").unwrap();

        let agents_dir = tmp.path().join("agents");
        let a1 = agents_dir.join("agent1");
        std::fs::create_dir_all(&a1).unwrap();
        std::fs::write(
            a1.join("config.yaml"),
            "name: agent1\ndescription: \"\"\nllm:\n  provider: kimi\n  model: history-model\ngoal: t\ntools: {}\nconstraints:\n  max_turns: 5\n  timeout_secs: 60\n",
        ).unwrap();

        // Must not panic, must fall back to history
        let candidates = candidates_for_provider(&home, &agents_dir, "kimi");
        assert!(
            candidates.contains(&"history-model".to_string()),
            "corrupt known_models.json must fall back to history-only candidates, got: {candidates:?}"
        );
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Sprint 12 M5 — /v1/models online probe (optional)
// ───────────────────────────────────────────────────────────────────────────

/// Probe a provider's `/v1/models` endpoint for its current live model
/// catalogue, returning a sorted unique list of model ids.
///
/// Delegates to `sage_runtime::llm::models::discover_models` and projects
/// the `DiscoveredModel` structs down to bare ids. Errors propagate
/// (HTTP / API error / parse) so the caller can surface them to the user.
///
/// Intended use (v0.0.2+): TUI hotkey 'p' in `sage init` refreshes the
/// candidate list by calling this, then merging results into
/// `~/.sage/known_models.json` via `record_used_model` (one entry per
/// discovered model).
///
/// `api_key` is optional — some local providers (Ollama / vLLM) don't
/// require auth on `/v1/models`. For cloud providers, pass
/// `Some(resolve_api_key_from_env(...))` per ProviderSpec.
///
/// Consumer: `sage init` TUI hotkey 'p' (task #82 pending).
#[allow(dead_code)]
pub async fn probe_provider_models(
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<String>, ProbeError> {
    let discovered =
        sage_runtime::llm::models::discover_models(base_url, api_key)
            .await
            .map_err(|e| ProbeError::Discover(e.to_string()))?;
    let mut ids: Vec<String> = discovered.into_iter().map(|m| m.id).collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

/// Error returned by [`probe_provider_models`]. Thin wrapper around the
/// underlying `DiscoveryError` to avoid leaking reqwest types across the
/// crate boundary.
#[allow(dead_code)]
#[derive(Debug)]
pub enum ProbeError {
    /// Any failure in the underlying HTTP / parse flow.
    Discover(String),
}

impl std::fmt::Display for ProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProbeError::Discover(msg) => write!(f, "probe failed: {msg}"),
        }
    }
}

impl std::error::Error for ProbeError {}

#[cfg(test)]
mod probe_tests {
    use super::*;

    #[tokio::test]
    async fn probe_error_display_contains_reason() {
        let e = ProbeError::Discover("HTTP 401 Unauthorized".to_string());
        let msg = e.to_string();
        assert!(msg.contains("probe failed"));
        assert!(msg.contains("401"));
    }

    #[tokio::test]
    async fn probe_unreachable_host_returns_discover_err() {
        // Use a port nothing listens on; reqwest will fail to connect.
        // This test proves the wire error is converted to ProbeError, not panicking.
        let result = probe_provider_models("http://127.0.0.1:1/v1", None).await;
        assert!(result.is_err(), "unreachable host must return Err");
        match result {
            Err(ProbeError::Discover(msg)) => {
                assert!(!msg.is_empty(), "error message should describe failure");
            }
            _ => panic!("expected ProbeError::Discover"),
        }
    }
}
