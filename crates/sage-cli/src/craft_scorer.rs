//! Offline craft efficiency scorer (Sprint 10 S10.3 lib layer).

#![allow(dead_code)] // wiring into CLI + cron is v1.0.2 follow-up

use std::collections::HashMap;
use std::path::Path;

/// Aggregated stats for a single craft, derived from multiple task records.
#[derive(Debug, Clone, PartialEq)]
pub struct CraftStats {
    pub usage_count: u32,
    pub tokens_total: u64,
    pub tokens_best: u64, // min input+output of any single run
    pub tokens_avg: u64,  // total / count
}

impl CraftStats {
    /// score = tokens_best / tokens_avg (≤ 1.0). Higher is better.
    /// `usage_count == 0` or `tokens_avg == 0` → 0.0 (undefined → 0).
    /// Clamped to 1.0 max (best cannot exceed average in a well-behaved scorer).
    pub fn score(&self) -> f32 {
        if self.usage_count == 0 || self.tokens_avg == 0 {
            return 0.0;
        }
        let raw = self.tokens_best as f32 / self.tokens_avg as f32;
        raw.min(1.0)
    }
}

/// A minimal projection of TaskRecord needed for scoring.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ScoringRecord {
    #[serde(default)]
    pub crafts_active: Vec<String>,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
}

/// Load all `metrics/*.json` (skipping `summary.json`) as `ScoringRecord`.
/// Malformed files are logged and skipped.
pub fn load_task_records(metrics_dir: &Path) -> Vec<ScoringRecord> {
    let entries = match std::fs::read_dir(metrics_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut records = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if path.file_name().and_then(|n| n.to_str()) == Some("summary.json") {
            continue;
        }
        let body = match std::fs::read_to_string(&path) {
            Ok(b) => b,
            Err(err) => {
                tracing::warn!("failed to read {:?}: {}", path, err);
                continue;
            }
        };
        match serde_json::from_str::<ScoringRecord>(&body) {
            Ok(r) => records.push(r),
            Err(err) => {
                tracing::warn!("failed to parse {:?}: {}", path, err);
            }
        }
    }
    records
}

/// Aggregate records by craft name: a record with `crafts_active = [a, b]`
/// contributes to both `a` and `b` (tokens_total += sum, usage_count += 1).
/// tokens_best tracks the min (input+output) across runs touching the craft.
pub fn aggregate_by_craft(records: &[ScoringRecord]) -> HashMap<String, CraftStats> {
    let mut map: HashMap<String, CraftStats> = HashMap::new();
    for record in records {
        if record.crafts_active.is_empty() {
            continue;
        }
        let tokens_sum = record.input_tokens + record.output_tokens;
        for craft in &record.crafts_active {
            let entry = map.entry(craft.clone()).or_insert(CraftStats {
                usage_count: 0,
                tokens_total: 0,
                tokens_best: u64::MAX,
                tokens_avg: 0,
            });
            entry.usage_count += 1;
            entry.tokens_total += tokens_sum;
            entry.tokens_best = entry.tokens_best.min(tokens_sum);
        }
    }
    for stats in map.values_mut() {
        if stats.usage_count > 0 {
            stats.tokens_avg = stats.tokens_total / stats.usage_count as u64;
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    // ── helpers ─────────────────────────────────────────────────────────────

    fn make_stats(usage_count: u32, tokens_total: u64, tokens_best: u64, tokens_avg: u64) -> CraftStats {
        CraftStats { usage_count, tokens_total, tokens_best, tokens_avg }
    }

    fn make_record(crafts: &[&str], input: u64, output: u64) -> ScoringRecord {
        ScoringRecord {
            crafts_active: crafts.iter().map(|s| s.to_string()).collect(),
            input_tokens: input,
            output_tokens: output,
        }
    }

    /// Write `content` to `dir/filename`, creating `dir` if needed.
    fn write_file(dir: &Path, filename: &str, content: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(filename), content).unwrap();
    }

    fn temp_dir(suffix: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("sage_test_craft_scorer_{suffix}"));
        let _ = fs::remove_dir_all(&base);
        base
    }

    // ── CraftStats::score ────────────────────────────────────────────────────

    #[test]
    fn score_zero_usage_returns_zero() {
        let s = make_stats(0, 0, 0, 100);
        assert_eq!(s.score(), 0.0);
    }

    #[test]
    fn score_zero_avg_returns_zero() {
        let s = make_stats(1, 0, 0, 0);
        assert_eq!(s.score(), 0.0);
    }

    #[test]
    fn score_best_equals_avg_returns_one() {
        let s = make_stats(1, 100, 100, 100);
        assert!((s.score() - 1.0).abs() < f32::EPSILON, "expected 1.0, got {}", s.score());
    }

    #[test]
    fn score_half_ratio_returns_half() {
        let s = make_stats(1, 100, 50, 100);
        assert!((s.score() - 0.5).abs() < f32::EPSILON, "expected 0.5, got {}", s.score());
    }

    /// tokens_best > tokens_avg is logically odd but possible if data is inconsistent.
    /// Spec says implementer decides; we lock the expected behavior: clamp to 1.0.
    #[test]
    fn score_best_gt_avg_clamps_to_one() {
        let s = make_stats(1, 150, 150, 100);
        assert!(
            (s.score() - 1.0).abs() < f32::EPSILON,
            "expected clamped 1.0, got {}",
            s.score()
        );
    }

    // ── load_task_records ────────────────────────────────────────────────────

    #[test]
    fn load_task_records_empty_dir_returns_empty() {
        let dir = temp_dir("empty");
        fs::create_dir_all(&dir).unwrap();
        let records = load_task_records(&dir);
        assert!(records.is_empty(), "expected empty vec, got {:?}", records);
    }

    #[test]
    fn load_task_records_missing_dir_returns_empty() {
        let dir = temp_dir("missing_does_not_exist_xyz");
        // Do NOT create the directory — it must not exist.
        assert!(!dir.exists());
        let records = load_task_records(&dir);
        assert!(records.is_empty(), "expected empty vec for missing dir");
    }

    #[test]
    fn load_task_records_reads_single_json() {
        let dir = temp_dir("single");
        write_file(
            &dir,
            "01ABC.json",
            r#"{"crafts_active":["a"],"input_tokens":100,"output_tokens":50}"#,
        );
        let records = load_task_records(&dir);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].crafts_active, vec!["a"]);
        assert_eq!(records[0].input_tokens, 100);
        assert_eq!(records[0].output_tokens, 50);
    }

    #[test]
    fn load_task_records_reads_multiple_json() {
        let dir = temp_dir("multi");
        write_file(
            &dir,
            "task1.json",
            r#"{"crafts_active":["x"],"input_tokens":10,"output_tokens":5}"#,
        );
        write_file(
            &dir,
            "task2.json",
            r#"{"crafts_active":["y"],"input_tokens":20,"output_tokens":10}"#,
        );
        let records = load_task_records(&dir);
        assert_eq!(records.len(), 2);
    }

    #[test]
    fn load_task_records_skips_summary_json() {
        let dir = temp_dir("summary");
        write_file(&dir, "summary.json", r#"{"crafts_active":["z"],"input_tokens":999,"output_tokens":1}"#);
        write_file(&dir, "task_a.json", r#"{"crafts_active":["a"],"input_tokens":1,"output_tokens":1}"#);
        let records = load_task_records(&dir);
        // Only task_a.json should be loaded, not summary.json.
        assert_eq!(records.len(), 1, "summary.json must be skipped");
        assert_eq!(records[0].crafts_active, vec!["a"]);
    }

    #[test]
    fn load_task_records_skips_malformed_json() {
        let dir = temp_dir("malformed");
        write_file(&dir, "bad.json", "this is not json {{{");
        write_file(&dir, "good.json", r#"{"crafts_active":["ok"],"input_tokens":1,"output_tokens":1}"#);
        // Should not panic; bad.json is skipped.
        let records = load_task_records(&dir);
        assert_eq!(records.len(), 1, "malformed json must be skipped, not panic");
    }

    #[test]
    fn load_task_records_skips_non_json_files() {
        let dir = temp_dir("non_json");
        write_file(&dir, "notes.txt", "not a json");
        write_file(&dir, "readme.md", "# readme");
        write_file(&dir, "task.json", r#"{"crafts_active":["a"],"input_tokens":1,"output_tokens":1}"#);
        let records = load_task_records(&dir);
        assert_eq!(records.len(), 1, ".txt and .md must be skipped");
    }

    #[test]
    fn load_task_records_tolerates_missing_crafts_active() {
        let dir = temp_dir("no_crafts");
        // No crafts_active field at all — serde default should give empty vec.
        write_file(&dir, "task.json", r#"{"input_tokens":42,"output_tokens":8}"#);
        let records = load_task_records(&dir);
        assert_eq!(records.len(), 1, "record should be loaded despite missing crafts_active");
        assert!(records[0].crafts_active.is_empty(), "missing crafts_active should default to []");
    }

    // ── aggregate_by_craft ───────────────────────────────────────────────────

    #[test]
    fn aggregate_empty_records_returns_empty_map() {
        let map = aggregate_by_craft(&[]);
        assert!(map.is_empty());
    }

    #[test]
    fn aggregate_single_record_single_craft() {
        let records = [make_record(&["deploy"], 100, 50)];
        let map = aggregate_by_craft(&records);
        assert_eq!(map.len(), 1);
        let stats = &map["deploy"];
        assert_eq!(stats.usage_count, 1);
        assert_eq!(stats.tokens_total, 150);
        assert_eq!(stats.tokens_best, 150);
        assert_eq!(stats.tokens_avg, 150);
    }

    #[test]
    fn aggregate_single_record_skips_empty_crafts_active() {
        let records = [make_record(&[], 100, 50)];
        let map = aggregate_by_craft(&records);
        assert!(map.is_empty(), "record with empty crafts_active contributes nothing");
    }

    #[test]
    fn aggregate_multi_records_same_craft_sums_tokens_and_counts() {
        // run 1: deploy  input=100 output=50  → sum=150
        // run 2: deploy  input=80  output=40  → sum=120
        // total=270, best=120 (min), avg=135
        let records = [
            make_record(&["deploy"], 100, 50),
            make_record(&["deploy"], 80, 40),
        ];
        let map = aggregate_by_craft(&records);
        let stats = &map["deploy"];
        assert_eq!(stats.usage_count, 2);
        assert_eq!(stats.tokens_total, 270);
        assert_eq!(stats.tokens_best, 120, "tokens_best should be min(150,120)=120");
        assert_eq!(stats.tokens_avg, 135, "tokens_avg = 270/2 = 135");
    }

    #[test]
    fn aggregate_record_with_multiple_crafts_attributes_to_all() {
        // crafts_active = ["a", "b"], tokens = 100+50 = 150
        // Both a and b should each get usage_count=1, tokens_total=150
        let records = [make_record(&["a", "b"], 100, 50)];
        let map = aggregate_by_craft(&records);
        assert_eq!(map.len(), 2, "both crafts must appear in map");
        for craft in &["a", "b"] {
            let stats = map.get(*craft).unwrap_or_else(|| panic!("craft {craft} missing from map"));
            assert_eq!(stats.usage_count, 1);
            assert_eq!(stats.tokens_total, 150);
            assert_eq!(stats.tokens_best, 150);
            assert_eq!(stats.tokens_avg, 150);
        }
    }

    #[test]
    fn aggregate_tokens_best_tracks_minimum_not_last() {
        // 3 runs of "deploy" with token sums: 200, 100, 150
        // tokens_best must be 100 (the minimum), not 150 (the last)
        let records = [
            make_record(&["deploy"], 120, 80),  // sum=200
            make_record(&["deploy"], 60, 40),   // sum=100  ← min
            make_record(&["deploy"], 90, 60),   // sum=150
        ];
        let map = aggregate_by_craft(&records);
        let stats = &map["deploy"];
        assert_eq!(stats.usage_count, 3);
        assert_eq!(stats.tokens_total, 450);
        assert_eq!(stats.tokens_best, 100, "tokens_best should track global minimum=100");
        assert_eq!(stats.tokens_avg, 150, "tokens_avg = 450/3 = 150");
    }
}
