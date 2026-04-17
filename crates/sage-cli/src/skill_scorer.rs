//! Offline craft efficiency scorer (Sprint 10 S10.3 lib layer).
//!
//! Wired into the CLI by Sprint 12 task #72 — `sage skill-score` calls
//! `load_task_records` → `aggregate_by_craft` → `format_skill_score_report`.
//! The `crafts_needing_evaluation` helper is still waiting on a
//! daemon-side scheduler (task #83) so its call site is absent for now.

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

/// Thresholds for triggering automatic SkillEvaluation session.
///
/// A craft needs both low efficiency (score < SCORE_THRESHOLD) AND enough
/// samples (usage_count >= MIN_USAGE) before the scheduler spawns a
/// rewrite session. The min-usage guard prevents a single bad run from
/// triggering wasted LLM calls on a brand-new craft.
pub const SCORE_THRESHOLD: f32 = 0.5;
pub const MIN_USAGE_FOR_EVALUATION: u32 = 5;

/// Return craft names that qualify for automatic SkillEvaluation session.
///
/// Selection criteria (both required):
///   - score() < SCORE_THRESHOLD (inefficient)
///   - usage_count >= MIN_USAGE_FOR_EVALUATION (enough samples)
///
/// Returned names are sorted alphabetically for deterministic scheduling.
pub fn crafts_needing_evaluation(
    stats: &std::collections::HashMap<String, CraftStats>,
) -> Vec<String> {
    let mut names: Vec<String> = stats
        .iter()
        .filter(|(_, s)| s.usage_count >= MIN_USAGE_FOR_EVALUATION && s.score() < SCORE_THRESHOLD)
        .map(|(n, _)| n.clone())
        .collect();
    names.sort();
    names
}

/// Render a human-readable report for `sage skill-score`.
///
/// Output format is fixed: a two-decimal score (`0.80`) so tests can grep
/// for it and operators can grep historic reports. Crafts are sorted by
/// name for deterministic, diff-friendly output. Sprint 12 task #72
/// sub-path 3.
///
/// When `needs_only` is `true`, only crafts returned by
/// [`crafts_needing_evaluation`] are shown (low-score + sufficient-usage).
/// Empty results produce a friendly "no data" / "no candidates" hint rather
/// than a blank table.
pub fn format_skill_score_report(
    stats: &HashMap<String, CraftStats>,
    needs_only: bool,
) -> String {
    if stats.is_empty() {
        return "no data yet — run the agent and let it invoke a craft \
                before scoring"
            .to_string();
    }

    // Filter first, then sort for deterministic output.
    let visible: Vec<(&String, &CraftStats)> = if needs_only {
        let needy = crafts_needing_evaluation(stats);
        stats
            .iter()
            .filter(|(name, _)| needy.iter().any(|n| n == *name))
            .collect()
    } else {
        stats.iter().collect()
    };

    if visible.is_empty() {
        // Only reachable with needs_only=true and no qualifying craft —
        // the all-empty-stats case is handled above.
        return "no crafts currently qualify for evaluation \
                (threshold: score < 0.5 and usage_count >= 5)"
            .to_string();
    }

    let mut sorted = visible;
    sorted.sort_by(|a, b| a.0.cmp(b.0));

    let mut out = String::new();
    out.push_str("craft               usage  tokens_best  tokens_avg  score\n");
    out.push_str("─────               ─────  ───────────  ──────────  ─────\n");
    for (name, s) in &sorted {
        // Task #84: `{name:<19}` only pads by char count, not terminal
        // display width. CJK / full-width craft names (e.g. "接入飞书")
        // take 2 columns per char, so char-count padding misaligns. We
        // truncate by display width and hand-pad with ASCII spaces.
        let trimmed = truncate_for_column(name, 19);
        let pad = 19usize.saturating_sub(unicode_display_width(&trimmed));
        let spaces = " ".repeat(pad);
        let line = format!(
            "{trimmed}{spaces} {usage:>5}  {best:>11}  {avg:>10}  {score:.2}\n",
            usage = s.usage_count,
            best = s.tokens_best,
            avg = s.tokens_avg,
            score = s.score(),
        );
        out.push_str(&line);
    }
    out
}

/// Terminal display width of `s` via `unicode-width` (task #84).
///
/// Wraps `UnicodeWidthStr::width` so callers don't import the crate. CJK /
/// full-width / emoji measure correctly; control chars measure 0.
fn unicode_display_width(s: &str) -> usize {
    use unicode_width::UnicodeWidthStr;
    s.width()
}

/// Clip a craft name to fit the report's first column without breaking
/// alignment. Task #84: truncation is by **display width** (CJK counted as
/// 2), not char count — so a name like "接入飞书客服" (12 columns / 6 chars)
/// truncates at the display-width boundary.
fn truncate_for_column(name: &str, max_width: usize) -> String {
    use unicode_width::UnicodeWidthChar;
    if unicode_display_width(name) <= max_width {
        return name.to_string();
    }
    // Reserve 1 column for the ellipsis.
    let budget = max_width.saturating_sub(1);
    let mut acc = String::new();
    let mut used = 0usize;
    for ch in name.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > budget {
            break;
        }
        acc.push(ch);
        used += w;
    }
    acc.push('…');
    acc
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

    // ── crafts_needing_evaluation ────────────────────────────────────────────

    #[test]
    fn crafts_needing_evaluation_empty_map_returns_empty() {
        let map = std::collections::HashMap::new();
        assert!(crafts_needing_evaluation(&map).is_empty());
    }

    #[test]
    fn crafts_needing_evaluation_low_score_high_usage_returns_name() {
        // score = 200/500 = 0.4 < 0.5 ✓, usage=10 >= 5 ✓
        let mut map = std::collections::HashMap::new();
        map.insert("foo".to_string(), CraftStats {
            usage_count: 10,
            tokens_total: 1000,
            tokens_best: 200,
            tokens_avg: 500,
        });
        assert_eq!(crafts_needing_evaluation(&map), vec!["foo"]);
    }

    #[test]
    fn crafts_needing_evaluation_low_score_low_usage_filtered_out() {
        // score = 0.3 < 0.5, but usage=3 < 5 → filtered
        let mut map = std::collections::HashMap::new();
        map.insert("foo".to_string(), CraftStats {
            usage_count: 3,
            tokens_total: 300,
            tokens_best: 90,
            tokens_avg: 300,
        });
        assert!(crafts_needing_evaluation(&map).is_empty());
    }

    #[test]
    fn crafts_needing_evaluation_high_score_high_usage_filtered_out() {
        // score = 800/1000 = 0.8 >= 0.5 → filtered
        let mut map = std::collections::HashMap::new();
        map.insert("bar".to_string(), CraftStats {
            usage_count: 10,
            tokens_total: 10000,
            tokens_best: 800,
            tokens_avg: 1000,
        });
        assert!(crafts_needing_evaluation(&map).is_empty());
    }

    #[test]
    fn crafts_needing_evaluation_exact_threshold_boundary() {
        // score = 500/1000 = 0.5 — strict < means NOT included
        let mut map = std::collections::HashMap::new();
        map.insert("exact".to_string(), CraftStats {
            usage_count: 10,
            tokens_total: 10000,
            tokens_best: 500,
            tokens_avg: 1000,
        });
        assert!(crafts_needing_evaluation(&map).is_empty(), "score == 0.5 must NOT qualify");
    }

    #[test]
    fn crafts_needing_evaluation_exact_usage_boundary() {
        // usage=5 exactly meets MIN_USAGE_FOR_EVALUATION (>= includes equal)
        let mut map = std::collections::HashMap::new();
        map.insert("borderline".to_string(), CraftStats {
            usage_count: 5,
            tokens_total: 2000,
            tokens_best: 200,
            tokens_avg: 400,  // score = 200/400 = 0.5 — wait, need < 0.5
            // Use tokens_best=199, tokens_avg=400 → 0.4975 < 0.5
        });
        // Override with correct values: score = 100/400 = 0.25 < 0.5
        let mut map2 = std::collections::HashMap::new();
        map2.insert("borderline".to_string(), CraftStats {
            usage_count: 5,
            tokens_total: 2000,
            tokens_best: 100,
            tokens_avg: 400,
        });
        assert_eq!(crafts_needing_evaluation(&map2), vec!["borderline"], "usage==5 must qualify");
    }

    #[test]
    fn crafts_needing_evaluation_returns_sorted_alphabetically() {
        // "zebra" and "alpha" both qualify — result must be ["alpha", "zebra"]
        let mut map = std::collections::HashMap::new();
        for name in &["zebra", "alpha"] {
            map.insert(name.to_string(), CraftStats {
                usage_count: 10,
                tokens_total: 1000,
                tokens_best: 100,
                tokens_avg: 400,  // score = 0.25 < 0.5
            });
        }
        assert_eq!(crafts_needing_evaluation(&map), vec!["alpha", "zebra"]);
    }

    #[test]
    fn crafts_needing_evaluation_multiple_qualifying_crafts() {
        let mut map = std::collections::HashMap::new();
        // "good" — high score, filtered out
        map.insert("good".to_string(), CraftStats {
            usage_count: 10,
            tokens_total: 1000,
            tokens_best: 900,
            tokens_avg: 1000,  // score = 0.9
        });
        // "bad1" — qualifies
        map.insert("bad1".to_string(), CraftStats {
            usage_count: 8,
            tokens_total: 800,
            tokens_best: 100,
            tokens_avg: 100,  // score = 1.0... need < 0.5
        });
        // Fix bad1: tokens_best=40, tokens_avg=100 → score=0.4
        map.insert("bad1".to_string(), CraftStats {
            usage_count: 8,
            tokens_total: 800,
            tokens_best: 40,
            tokens_avg: 100,
        });
        // "bad2" — qualifies
        map.insert("bad2".to_string(), CraftStats {
            usage_count: 6,
            tokens_total: 600,
            tokens_best: 50,
            tokens_avg: 200,  // score = 0.25
        });
        let result = crafts_needing_evaluation(&map);
        assert_eq!(result, vec!["bad1", "bad2"], "both qualifying crafts returned, sorted");
    }

    // ── Sprint 12 task #72 sub-path 3: format_skill_score_report ──────────

    #[test]
    fn format_skill_score_report_empty_stats_shows_no_data_message() {
        // User guidance: the `sage skill-score` CLI must not return a scary
        // error when metrics haven't been collected yet. A newly initialised
        // agent has zero records; the tool should print a friendly hint
        // instead of an empty table.
        let stats: HashMap<String, CraftStats> = HashMap::new();
        let report = format_skill_score_report(&stats, false);
        assert!(
            report.contains("no data"),
            "empty report must mention 'no data', got: {report:?}"
        );
    }

    #[test]
    fn format_skill_score_report_lists_each_craft_with_score() {
        // Happy path: two crafts with known stats produce a report that
        // includes both names and their numeric score.
        let mut stats = HashMap::new();
        stats.insert(
            "alpha".to_string(),
            make_stats(10, 500, 40, 50),
        );
        stats.insert(
            "beta".to_string(),
            make_stats(3, 300, 80, 100),
        );
        let report = format_skill_score_report(&stats, false);
        assert!(report.contains("alpha"), "report must include craft name 'alpha'");
        assert!(report.contains("beta"), "report must include craft name 'beta'");
        // alpha score = 40/50 = 0.80; beta = 80/100 = 0.80.
        assert!(
            report.contains("0.80"),
            "report must render a score like 0.80 for both crafts, got: {report}"
        );
    }

    #[test]
    fn format_skill_score_report_sorts_crafts_deterministically() {
        // HashMap iteration is non-deterministic — the report must sort by
        // name so the CLI output is stable across invocations (diff-friendly
        // for operators comparing weeks of data).
        let mut stats = HashMap::new();
        stats.insert("zzz".into(), make_stats(5, 100, 10, 20));
        stats.insert("aaa".into(), make_stats(5, 100, 10, 20));
        stats.insert("mmm".into(), make_stats(5, 100, 10, 20));
        let report = format_skill_score_report(&stats, false);
        let aaa_pos = report.find("aaa").unwrap();
        let mmm_pos = report.find("mmm").unwrap();
        let zzz_pos = report.find("zzz").unwrap();
        assert!(
            aaa_pos < mmm_pos && mmm_pos < zzz_pos,
            "crafts must appear in lexicographic order; got positions aaa={aaa_pos}, mmm={mmm_pos}, zzz={zzz_pos}"
        );
    }

    #[test]
    fn format_skill_score_report_needs_only_filters_to_evaluation_candidates() {
        // `--needs-evaluation` flag: report should list only crafts that
        // pass `crafts_needing_evaluation` (score < 0.5 AND usage >= 5).
        let mut stats = HashMap::new();
        // needs eval: score = 10/100 = 0.10 < 0.5, usage 10 >= 5
        stats.insert("lazy".into(), make_stats(10, 1000, 10, 100));
        // doesn't need eval: high score
        stats.insert("good".into(), make_stats(10, 1000, 80, 100));
        // doesn't need eval: low usage
        stats.insert("new".into(), make_stats(2, 200, 10, 100));

        let report = format_skill_score_report(&stats, /* needs_only */ true);
        assert!(
            report.contains("lazy"),
            "needs-eval report must include 'lazy' (low score + sufficient usage)"
        );
        assert!(
            !report.contains("good"),
            "needs-eval report must NOT include 'good' (high score)"
        );
        assert!(
            !report.contains("new"),
            "needs-eval report must NOT include 'new' (insufficient usage)"
        );
    }

    #[test]
    fn format_skill_score_report_needs_only_with_zero_candidates_is_empty_hint() {
        // When no craft qualifies for evaluation, the filtered report should
        // explicitly say so (not print a blank body the user can't interpret).
        let mut stats = HashMap::new();
        stats.insert("good1".into(), make_stats(10, 1000, 80, 100));
        stats.insert("good2".into(), make_stats(10, 1000, 90, 100));
        let report = format_skill_score_report(&stats, true);
        // Report mentions "no" (as in "no crafts need evaluation" or
        // "no candidates") and does not list the healthy crafts.
        assert!(!report.contains("good1"));
        assert!(!report.contains("good2"));
        assert!(
            report.to_lowercase().contains("no"),
            "empty needs-eval report must include an explicit 'no' hint, got: {report:?}"
        );
    }

    // ── Task #84: CJK / full-width column alignment ──────────────────────

    #[test]
    fn truncate_for_column_leaves_pure_ascii_under_budget_untouched() {
        assert_eq!(truncate_for_column("alpha", 19), "alpha");
    }

    #[test]
    fn truncate_for_column_counts_cjk_as_two_columns_for_truncation_boundary() {
        // "接入飞书" is 4 chars × 2-column = 8 display width. Under 19 it
        // must not be truncated.
        let name = "接入飞书";
        assert_eq!(unicode_display_width(name), 8);
        assert_eq!(truncate_for_column(name, 19), name);
    }

    #[test]
    fn truncate_for_column_truncates_at_display_width_with_ellipsis() {
        // "接入飞书客服工具" = 8 chars × 2-column = 16 display. Budget 10
        // → reserve 1 for "…" → 9 display columns of body → fits 4 CJK
        // chars (8 display) — truncated string is "接入飞书…" with total
        // display width 9.
        let name = "接入飞书客服工具";
        let out = truncate_for_column(name, 10);
        assert!(out.ends_with('…'), "ellipsis must be tail marker");
        assert!(
            unicode_display_width(&out) <= 10,
            "truncated width must fit max_width, got {} for {:?}",
            unicode_display_width(&out),
            out
        );
    }

    #[test]
    fn format_skill_score_report_cjk_name_column_stays_aligned() {
        // Two rows, one ASCII one CJK — their usage columns must start at
        // the same column position (20-21st display column counting from
        // start of line). We test by finding the "usage" column header in
        // the header row vs a row's usage digit alignment.
        let mut stats = HashMap::new();
        stats.insert("alpha".into(), make_stats(5, 500, 50, 100));
        stats.insert("接入飞书".into(), make_stats(7, 700, 60, 100));
        let report = format_skill_score_report(&stats, false);
        // Every data line must contain the usage count at the same display
        // column position — compare the display width of the line's prefix
        // up to the first digit of usage across the two data rows.
        let lines: Vec<&str> = report.lines().collect();
        // Skip 2 header lines
        let row_alpha = lines.iter().find(|l| l.contains("alpha")).unwrap();
        let row_cjk = lines.iter().find(|l| l.contains("接入飞书")).unwrap();
        // First digit position (in display columns) must match.
        let prefix_width = |line: &str| -> usize {
            use unicode_width::UnicodeWidthChar;
            let mut w = 0;
            for ch in line.chars() {
                if ch.is_ascii_digit() {
                    return w;
                }
                w += ch.width().unwrap_or(0);
            }
            w
        };
        assert_eq!(
            prefix_width(row_alpha),
            prefix_width(row_cjk),
            "CJK row must align with ASCII row — got alpha={} cjk={} in:\n{report}",
            prefix_width(row_alpha),
            prefix_width(row_cjk)
        );
    }
}
