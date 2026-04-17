// Harness runner — evaluates agent outputs by running test suites with eval scripts.

use anyhow::{Context as _, Result};
use sage_runtime::event::{AgentEvent, AgentEventSink};
use sage_runtime::types::AgentMessage;
use serde::{Deserialize, Serialize};
use std::time::Instant;
use tokio::sync::Mutex;

// ── Criterion ─────────────────────────────────────────────────────────────────

/// Declarative pass/fail criterion for a test case.
#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "check", rename_all = "snake_case")]
pub enum Criterion {
    /// Last assistant output must match `pattern` (regex).
    OutputContains { pattern: String },
    /// At least one tool call with this name must have occurred.
    ToolCalled { tool: String },
    /// Token usage must be within budget.
    TokenBudget {
        max_input_tokens: Option<u64>,
        max_output_tokens: Option<u64>,
    },
    /// Number of agent turns must not exceed `max_turns`.
    TurnBudget { max_turns: u32 },
    /// No agent-level error must have occurred.
    NoError,
}

// ── Test suite YAML schema ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TestCase {
    pub name: String,
    pub message: String,
    /// Optional path to eval script. If absent and criteria is empty, always Pass.
    pub eval: Option<String>,
    /// Optional per-case max turns override (currently advisory; future use).
    pub max_turns: Option<usize>,
    /// Declarative criteria evaluated before the eval script (if any).
    #[serde(default)]
    pub criteria: Vec<Criterion>,
}

#[derive(Debug, Deserialize)]
pub struct TestSuite {
    pub suite: String,
    pub agent: String,
    pub cases: Vec<TestCase>,
}

// ── Results ───────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(tag = "status", content = "reason")]
pub enum CaseResult {
    Pass,
    Fail(String),
    Error(String),
}

#[derive(Debug, Serialize)]
pub struct TestOutcome {
    pub case_name: String,
    pub result: CaseResult,
    pub turns: u32,
    pub duration_ms: u64,
}

// ── Reporter ──────────────────────────────────────────────────────────────────

pub enum Reporter {
    Terminal,
    Json,
    Junit,
}

fn report_terminal(outcomes: &[TestOutcome]) {
    for o in outcomes {
        let elapsed = format_elapsed(o.duration_ms);
        match &o.result {
            CaseResult::Pass => {
                println!("PASS  {}  ({} turns, {})", o.case_name, o.turns, elapsed);
            }
            CaseResult::Fail(reason) => {
                println!("FAIL  {}  → {}", o.case_name, reason);
            }
            CaseResult::Error(reason) => {
                println!("ERROR {}  → {}", o.case_name, reason);
            }
        }
    }
}

fn format_elapsed(ms: u64) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else {
        format!("{:.1}s", ms as f64 / 1000.0)
    }
}

fn report_json(outcomes: &[TestOutcome]) {
    match serde_json::to_string_pretty(outcomes) {
        Ok(json) => println!("{json}"),
        Err(e) => eprintln!("sage harness: failed to serialize outcomes: {e}"),
    }
}

/// Escape special XML characters in attribute values and text content.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Write JUnit XML to `<output_dir>/<suite_name>.xml`.
///
/// Creates the directory if it does not exist.
pub fn report_junit(
    suite_name: &str,
    outcomes: &[TestOutcome],
    output_dir: &str,
) -> Result<()> {
    let total = outcomes.len();
    let failures = outcomes
        .iter()
        .filter(|o| matches!(o.result, CaseResult::Fail(_)))
        .count();
    let errors = outcomes
        .iter()
        .filter(|o| matches!(o.result, CaseResult::Error(_)))
        .count();
    let total_ms: u64 = outcomes.iter().map(|o| o.duration_ms).sum();
    let total_secs = total_ms as f64 / 1000.0;

    // Build XML by string concatenation — no extra crate needed.
    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str(&format!("<testsuites time=\"{:.1}\">\n", total_secs));
    xml.push_str(&format!(
        "  <testsuite name=\"{}\" tests=\"{}\" failures=\"{}\" errors=\"{}\" time=\"{:.1}\">\n",
        xml_escape(suite_name),
        total,
        failures,
        errors,
        total_secs,
    ));

    for o in outcomes {
        let secs = o.duration_ms as f64 / 1000.0;
        // classname derived from suite name (dots replaced)
        let classname = xml_escape(&suite_name.replace('-', ".").replace('_', "."));
        match &o.result {
            CaseResult::Pass => {
                xml.push_str(&format!(
                    "    <testcase name=\"{}\" time=\"{:.1}\" classname=\"{}\" />\n",
                    xml_escape(&o.case_name),
                    secs,
                    classname,
                ));
            }
            CaseResult::Fail(reason) => {
                xml.push_str(&format!(
                    "    <testcase name=\"{}\" time=\"{:.1}\" classname=\"{}\">\n",
                    xml_escape(&o.case_name),
                    secs,
                    classname,
                ));
                xml.push_str(&format!(
                    "      <failure message=\"{}\"/>\n",
                    xml_escape(reason),
                ));
                xml.push_str("    </testcase>\n");
            }
            CaseResult::Error(reason) => {
                xml.push_str(&format!(
                    "    <testcase name=\"{}\" time=\"{:.1}\" classname=\"{}\">\n",
                    xml_escape(&o.case_name),
                    secs,
                    classname,
                ));
                xml.push_str(&format!(
                    "      <error message=\"{}\"/>\n",
                    xml_escape(reason),
                ));
                xml.push_str("    </testcase>\n");
            }
        }
    }

    xml.push_str("  </testsuite>\n");
    xml.push_str("</testsuites>\n");

    // Write to file — create dir if needed.
    let dir = std::path::Path::new(output_dir);
    std::fs::create_dir_all(dir)
        .with_context(|| format!("cannot create output directory: {output_dir}"))?;

    // Sanitise suite name for use in filename (replace spaces and path seps).
    let filename = suite_name
        .replace('/', "_")
        .replace('\\', "_")
        .replace(' ', "_");
    let path = dir.join(format!("{filename}.xml"));
    std::fs::write(&path, xml)
        .with_context(|| format!("cannot write JUnit XML to {}", path.display()))?;

    Ok(())
}

// ── evaluate_criteria ─────────────────────────────────────────────────────────

/// Evaluate declarative criteria against a completed agent run.
///
/// Pure function — does not touch any engine or session state.
///
/// # Parameters
/// - `criteria`       — slice of criteria to evaluate (in order)
/// - `last_output`    — last assistant text message
/// - `tool_calls`     — names of all tools called during the run (in order)
/// - `turns`          — number of turns taken
/// - `input_tokens`   — total input tokens consumed
/// - `output_tokens`  — total output tokens produced
/// - `error`          — agent-level error string, if any
///
/// Returns `CaseResult::Pass` when all criteria pass, or the first
/// `CaseResult::Fail` whose message describes the failing criterion.
/// Returns `CaseResult::Pass` immediately when `criteria` is empty.
pub fn evaluate_criteria(
    criteria: &[Criterion],
    last_output: &str,
    tool_calls: &[String],
    turns: u32,
    input_tokens: u64,
    output_tokens: u64,
    error: Option<&str>,
) -> CaseResult {
    for criterion in criteria {
        let result = evaluate_single(
            criterion,
            last_output,
            tool_calls,
            turns,
            input_tokens,
            output_tokens,
            error,
        );
        if !matches!(result, CaseResult::Pass) {
            return result;
        }
    }
    CaseResult::Pass
}

fn evaluate_single(
    criterion: &Criterion,
    last_output: &str,
    tool_calls: &[String],
    turns: u32,
    input_tokens: u64,
    output_tokens: u64,
    error: Option<&str>,
) -> CaseResult {
    match criterion {
        Criterion::OutputContains { pattern } => {
            match regex::Regex::new(pattern) {
                Err(e) => CaseResult::Fail(format!("invalid regex pattern {:?}: {e}", pattern)),
                Ok(re) => {
                    if re.is_match(last_output) {
                        CaseResult::Pass
                    } else {
                        CaseResult::Fail(format!(
                            "output_contains: pattern {:?} not found in output",
                            pattern
                        ))
                    }
                }
            }
        }
        Criterion::ToolCalled { tool: _ } => {
            // ToolCalled criterion: not yet available, blocked until Sprint 10.
            // Tool call tracing is not yet wired from the agent session event stream.
            CaseResult::Error(
                "ToolCalled criterion not yet wired to agent tool trace; blocked in Sprint 9; will enable in Sprint 10".to_string()
            )
        }
        Criterion::TokenBudget {
            max_input_tokens,
            max_output_tokens,
        } => {
            if let Some(max_in) = max_input_tokens {
                if input_tokens > *max_in {
                    return CaseResult::Fail(format!(
                        "token_budget: input tokens exceeded: expected max {max_in}, got {input_tokens}"
                    ));
                }
            }
            if let Some(max_out) = max_output_tokens {
                if output_tokens > *max_out {
                    return CaseResult::Fail(format!(
                        "token_budget: output tokens exceeded: expected max {max_out}, got {output_tokens}"
                    ));
                }
            }
            CaseResult::Pass
        }
        Criterion::TurnBudget { max_turns } => {
            if turns > *max_turns {
                CaseResult::Fail(format!(
                    "turn_budget: turns exceeded: expected max {max_turns}, got {turns}"
                ))
            } else {
                CaseResult::Pass
            }
        }
        Criterion::NoError => match error {
            None => CaseResult::Pass,
            Some(e) => CaseResult::Fail(format!("no_error: agent error occurred: {e}")),
        },
    }
}

// ── case_filter ───────────────────────────────────────────────────────────────

/// Returns `true` if `case_name` matches `filter`.
///
/// `filter` supports glob-style wildcards: `*` matches any sequence of
/// characters, `?` matches a single character. Matching is case-sensitive.
/// If `filter` contains neither `*` nor `?` it is treated as an exact match.
pub fn case_matches_filter(case_name: &str, filter: &str) -> bool {
    // Convert glob pattern to regex.
    let mut re_pattern = String::from("^");
    for ch in filter.chars() {
        match ch {
            '*' => re_pattern.push_str(".*"),
            '?' => re_pattern.push('.'),
            c => {
                // Escape regex meta-characters.
                for escaped_ch in regex::escape(&c.to_string()).chars() {
                    re_pattern.push(escaped_ch);
                }
            }
        }
    }
    re_pattern.push('$');
    regex::Regex::new(&re_pattern)
        .map(|re| re.is_match(case_name))
        .unwrap_or(false)
}

/// Filter a list of cases by an optional glob/exact filter.
///
/// Returns all cases when `filter` is `None`.
pub fn filter_cases<'a>(cases: &'a [TestCase], filter: Option<&str>) -> Vec<&'a TestCase> {
    match filter {
        None => cases.iter().collect(),
        Some(f) => cases
            .iter()
            .filter(|c| case_matches_filter(&c.name, f))
            .collect(),
    }
}

// ── Turn counter sink ─────────────────────────────────────────────────────────

/// Wraps `CollectSink` and counts TurnStart events to track turn usage.
struct TurnCountingSink {
    turns: Mutex<u32>,
}

impl TurnCountingSink {
    fn new() -> Self {
        Self {
            turns: Mutex::new(0),
        }
    }

    async fn turn_count(&self) -> u32 {
        *self.turns.lock().await
    }
}

#[async_trait::async_trait]
impl AgentEventSink for TurnCountingSink {
    async fn emit(&self, event: AgentEvent) {
        if matches!(event, AgentEvent::TurnStart) {
            let mut guard = self.turns.lock().await;
            *guard += 1;
        }
    }
}

// ── Eval script ───────────────────────────────────────────────────────────────

/// Run the eval script with the given env vars.
///
/// Exit codes:
///   0 → Pass
///   2 → Fail (reads stderr for reason)
///   other → Error
async fn run_eval_script(
    script: &str,
    last_message: &str,
    agent_name: &str,
    case_name: &str,
) -> CaseResult {
    let output = match tokio::process::Command::new("sh")
        .arg("-c")
        .arg(script)
        .env("SAGE_LAST_MESSAGE", last_message)
        .env("SAGE_AGENT", agent_name)
        .env("SAGE_CASE", case_name)
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => return CaseResult::Error(format!("failed to spawn eval script: {e}")),
    };

    match output.status.code() {
        Some(0) => CaseResult::Pass,
        Some(2) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let reason = if stderr.is_empty() {
                "eval script exited with code 2 (no stderr)".to_string()
            } else {
                format!("eval script: {stderr}")
            };
            CaseResult::Fail(reason)
        }
        Some(code) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            CaseResult::Error(format!(
                "eval script exited with code {code}{}",
                if stderr.is_empty() {
                    String::new()
                } else {
                    format!(": {stderr}")
                }
            ))
        }
        None => CaseResult::Error("eval script killed by signal".to_string()),
    }
}

// ── Extract last assistant text ───────────────────────────────────────────────

fn last_assistant_text(messages: &[AgentMessage]) -> String {
    messages
        .iter()
        .rev()
        .find_map(|m| {
            if let AgentMessage::Assistant(a) = m {
                let text = a.text();
                if !text.is_empty() {
                    return Some(text);
                }
            }
            None
        })
        .unwrap_or_default()
}

// ── run_single_case ───────────────────────────────────────────────────────────

/// Run a single test case and return the outcome.
///
/// Builds a fresh engine so cases are fully isolated.
async fn run_single_case(
    case: &TestCase,
    config: &sage_runner::config::AgentConfig,
    suite_agent: &str,
) -> TestOutcome {
    let start = Instant::now();

    let engine = match crate::serve::build_engine_for_agent(config, true).await {
        Ok(e) => e,
        Err(e) => {
            return TestOutcome {
                case_name: case.name.clone(),
                result: CaseResult::Error(format!("failed to build engine: {e}")),
                turns: 0,
                duration_ms: start.elapsed().as_millis() as u64,
            };
        }
    };

    let mut session = match engine.session().await {
        Ok(s) => s,
        Err(e) => {
            return TestOutcome {
                case_name: case.name.clone(),
                result: CaseResult::Error(format!("failed to create session: {e}")),
                turns: 0,
                duration_ms: start.elapsed().as_millis() as u64,
            };
        }
    };

    let sink = TurnCountingSink::new();
    let send_result = session.send(&case.message, &sink).await;
    let turns = sink.turn_count().await;
    let duration_ms = start.elapsed().as_millis() as u64;

    let error_str: Option<String> = send_result.as_ref().err().map(|e| e.to_string());

    // Agent-level error short-circuits immediately — regardless of criteria.
    if let Some(ref err) = error_str {
        return TestOutcome {
            case_name: case.name.clone(),
            result: CaseResult::Error(format!("agent error: {err}")),
            turns,
            duration_ms,
        };
    }

    let last_text = last_assistant_text(session.messages());

    // Evaluate declarative criteria first (if any).
    let criteria_result = if !case.criteria.is_empty() {
        evaluate_criteria(
            &case.criteria,
            &last_text,
            &[], // tool_calls: not yet wired; tests mock this
            turns,
            0,
            0,
            None,
        )
    } else {
        CaseResult::Pass
    };

    // If criteria failed (or we already have an error result), short-circuit.
    if !matches!(criteria_result, CaseResult::Pass) {
        return TestOutcome {
            case_name: case.name.clone(),
            result: criteria_result,
            turns,
            duration_ms,
        };
    }

    // Run eval script if present.
    let result = match &case.eval {
        None => CaseResult::Pass,
        Some(script) => run_eval_script(script, &last_text, suite_agent, &case.name).await,
    };

    TestOutcome {
        case_name: case.name.clone(),
        result,
        turns,
        duration_ms,
    }
}

// ── run_test_suite ────────────────────────────────────────────────────────────

/// Load and run a test suite YAML file.
///
/// Returns `true` if all cases passed, `false` if any failed or errored.
///
/// # Parameters
/// - `suite_path`   — path to the test suite YAML
/// - `reporter`     — output format
/// - `case_filter`  — optional glob/exact filter; `None` runs all cases
/// - `parallel`     — max concurrent cases (1 = serial / original behaviour)
/// - `output_dir`   — directory for JUnit XML (only used with `Reporter::Junit`)
pub async fn run_test_suite(
    suite_path: &str,
    reporter: Reporter,
    case_filter: Option<&str>,
    parallel: usize,
    output_dir: Option<&str>,
) -> Result<bool> {
    // Load YAML
    let yaml = tokio::fs::read_to_string(suite_path)
        .await
        .with_context(|| format!("cannot read test suite at {suite_path}"))?;
    let suite: TestSuite =
        serde_yaml::from_str(&yaml).with_context(|| format!("invalid test suite YAML: {suite_path}"))?;

    tracing::info!(suite = %suite.suite, agent = %suite.agent, cases = suite.cases.len(), "running test suite");

    // Load agent config
    let config = crate::serve::load_agent_config(&suite.agent)
        .await
        .with_context(|| format!("cannot load agent config for '{}'", suite.agent))?;

    // Apply case filter
    let filtered = filter_cases(&suite.cases, case_filter);
    tracing::info!(
        filtered = filtered.len(),
        filter = ?case_filter,
        "case filter applied"
    );

    // Run cases — serial or parallel, preserving input order.
    let parallel = parallel.max(1);
    let outcomes: Vec<TestOutcome> = {
        use futures::stream::{self, StreamExt};
        let mut indexed: Vec<(usize, TestOutcome)> = stream::iter(
            filtered.into_iter().enumerate().map(|(idx, case)| {
                let config_ref = &config;
                let agent_name = suite.agent.as_str();
                async move {
                    let outcome = run_single_case(case, config_ref, agent_name).await;
                    (idx, outcome)
                }
            }),
        )
        .buffer_unordered(parallel)
        .collect()
        .await;
        indexed.sort_by_key(|(idx, _)| *idx);
        indexed.into_iter().map(|(_, o)| o).collect()
    };

    // Report
    match reporter {
        Reporter::Terminal => report_terminal(&outcomes),
        Reporter::Json => report_json(&outcomes),
        Reporter::Junit => {
            let dir = output_dir.unwrap_or(".");
            report_junit(&suite.suite, &outcomes, dir)?;
        }
    }

    let all_pass = outcomes.iter().all(|o| matches!(o.result, CaseResult::Pass));
    Ok(all_pass)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── helpers ───────────────────────────────────────────────────────────────

    fn no_tools() -> Vec<String> {
        vec![]
    }

    fn tools(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    // ════════════════════════════════════════════════════════════════════════
    // Criterion YAML deserialization
    // ════════════════════════════════════════════════════════════════════════

    #[test]
    fn criterion_output_contains_parses() {
        let yaml = r#"check: output_contains
pattern: "会议|日历""#;
        let c: Criterion = serde_yaml::from_str(yaml).expect("parse output_contains");
        assert!(matches!(c, Criterion::OutputContains { .. }));
        if let Criterion::OutputContains { pattern } = c {
            assert_eq!(pattern, "会议|日历");
        }
    }

    #[test]
    fn criterion_tool_called_parses() {
        let yaml = r#"check: tool_called
tool: "bash""#;
        let c: Criterion = serde_yaml::from_str(yaml).expect("parse tool_called");
        assert!(matches!(c, Criterion::ToolCalled { .. }));
        if let Criterion::ToolCalled { tool } = c {
            assert_eq!(tool, "bash");
        }
    }

    #[test]
    fn criterion_token_budget_full_parses() {
        let yaml = r#"check: token_budget
max_input_tokens: 2000
max_output_tokens: 500"#;
        let c: Criterion = serde_yaml::from_str(yaml).expect("parse token_budget full");
        if let Criterion::TokenBudget {
            max_input_tokens,
            max_output_tokens,
        } = c
        {
            assert_eq!(max_input_tokens, Some(2000));
            assert_eq!(max_output_tokens, Some(500));
        } else {
            panic!("expected TokenBudget");
        }
    }

    #[test]
    fn criterion_token_budget_only_max_input_parses() {
        let yaml = r#"check: token_budget
max_input_tokens: 1000"#;
        let c: Criterion = serde_yaml::from_str(yaml).expect("parse token_budget only input");
        if let Criterion::TokenBudget {
            max_input_tokens,
            max_output_tokens,
        } = c
        {
            assert_eq!(max_input_tokens, Some(1000));
            assert_eq!(max_output_tokens, None);
        } else {
            panic!("expected TokenBudget");
        }
    }

    #[test]
    fn criterion_turn_budget_parses() {
        let yaml = r#"check: turn_budget
max_turns: 5"#;
        let c: Criterion = serde_yaml::from_str(yaml).expect("parse turn_budget");
        if let Criterion::TurnBudget { max_turns } = c {
            assert_eq!(max_turns, 5);
        } else {
            panic!("expected TurnBudget");
        }
    }

    #[test]
    fn criterion_no_error_parses() {
        let yaml = r#"check: no_error"#;
        let c: Criterion = serde_yaml::from_str(yaml).expect("parse no_error");
        assert!(matches!(c, Criterion::NoError));
    }

    #[test]
    fn test_case_with_mixed_criteria_parses() {
        let yaml = r#"
name: "查询今日日历"
message: "帮我查今天有哪些会议"
criteria:
  - { check: output_contains, pattern: "会议|日历|日程" }
  - { check: tool_called, tool: "bash" }
  - { check: token_budget, max_input_tokens: 2000, max_output_tokens: 500 }
  - { check: turn_budget, max_turns: 5 }
  - { check: no_error }
eval: "optional/script.sh"
"#;
        let tc: TestCase = serde_yaml::from_str(yaml).expect("parse mixed criteria test case");
        assert_eq!(tc.name, "查询今日日历");
        assert_eq!(tc.criteria.len(), 5);
        assert_eq!(tc.eval.as_deref(), Some("optional/script.sh"));
        assert!(matches!(tc.criteria[0], Criterion::OutputContains { .. }));
        assert!(matches!(tc.criteria[1], Criterion::ToolCalled { .. }));
        assert!(matches!(tc.criteria[2], Criterion::TokenBudget { .. }));
        assert!(matches!(tc.criteria[3], Criterion::TurnBudget { .. }));
        assert!(matches!(tc.criteria[4], Criterion::NoError));
    }

    // ════════════════════════════════════════════════════════════════════════
    // evaluate_criteria — pure function tests
    // ════════════════════════════════════════════════════════════════════════

    // ── output_contains ───────────────────────────────────────────────────

    #[test]
    fn eval_output_contains_hit_passes() {
        let criteria = vec![Criterion::OutputContains {
            pattern: "会议|日历".to_string(),
        }];
        let result = evaluate_criteria(&criteria, "今天有三个会议", &no_tools(), 1, 100, 50, None);
        assert!(matches!(result, CaseResult::Pass));
    }

    #[test]
    fn eval_output_contains_miss_fails_with_pattern_in_reason() {
        let criteria = vec![Criterion::OutputContains {
            pattern: "xyz_unique".to_string(),
        }];
        let result = evaluate_criteria(&criteria, "今天天气很好", &no_tools(), 1, 100, 50, None);
        match result {
            CaseResult::Fail(reason) => {
                assert!(
                    reason.contains("xyz_unique"),
                    "reason should mention pattern, got: {reason}"
                );
                assert!(
                    reason.contains("output_contains"),
                    "reason should mention criterion, got: {reason}"
                );
            }
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    // ── tool_called ───────────────────────────────────────────────────────

    #[test]
    fn eval_tool_called_returns_error_not_yet_wired() {
        // ToolCalled is blocked until Sprint 10; always returns Error to surface
        // the issue immediately rather than silently passing or failing.
        let criteria = vec![Criterion::ToolCalled {
            tool: "bash".to_string(),
        }];
        let result =
            evaluate_criteria(&criteria, "done", &tools(&["bash"]), 1, 100, 50, None);
        match result {
            CaseResult::Error(msg) => {
                assert!(msg.contains("ToolCalled"), "should mention criterion: {msg}");
                assert!(msg.contains("Sprint 10"), "should mention Sprint 10: {msg}");
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn eval_tool_called_absent_also_returns_error() {
        // Even when the tool was NOT called, ToolCalled returns Error (not wired yet).
        let criteria = vec![Criterion::ToolCalled {
            tool: "bash".to_string(),
        }];
        let result = evaluate_criteria(&criteria, "done", &tools(&["read_file"]), 1, 100, 50, None);
        assert!(matches!(result, CaseResult::Error(_)));
    }

    // ── token_budget ──────────────────────────────────────────────────────

    #[test]
    fn eval_token_budget_within_passes() {
        let criteria = vec![Criterion::TokenBudget {
            max_input_tokens: Some(2000),
            max_output_tokens: Some(500),
        }];
        let result = evaluate_criteria(&criteria, "", &no_tools(), 1, 1999, 499, None);
        assert!(matches!(result, CaseResult::Pass));
    }

    #[test]
    fn eval_token_budget_input_exceeded_fails() {
        let criteria = vec![Criterion::TokenBudget {
            max_input_tokens: Some(2000),
            max_output_tokens: Some(500),
        }];
        let result = evaluate_criteria(&criteria, "", &no_tools(), 1, 2001, 400, None);
        match result {
            CaseResult::Fail(reason) => {
                assert!(
                    reason.contains("input tokens exceeded"),
                    "got: {reason}"
                );
                assert!(reason.contains("2000"), "got: {reason}");
                assert!(reason.contains("2001"), "got: {reason}");
            }
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    #[test]
    fn eval_token_budget_output_exceeded_fails() {
        let criteria = vec![Criterion::TokenBudget {
            max_input_tokens: Some(2000),
            max_output_tokens: Some(500),
        }];
        let result = evaluate_criteria(&criteria, "", &no_tools(), 1, 1000, 600, None);
        match result {
            CaseResult::Fail(reason) => {
                assert!(
                    reason.contains("output tokens exceeded"),
                    "got: {reason}"
                );
                assert!(reason.contains("500"), "got: {reason}");
                assert!(reason.contains("600"), "got: {reason}");
            }
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    // ── turn_budget ───────────────────────────────────────────────────────

    #[test]
    fn eval_turn_budget_within_passes() {
        let criteria = vec![Criterion::TurnBudget { max_turns: 5 }];
        let result = evaluate_criteria(&criteria, "", &no_tools(), 4, 0, 0, None);
        assert!(matches!(result, CaseResult::Pass));
    }

    #[test]
    fn eval_turn_budget_exceeded_fails() {
        let criteria = vec![Criterion::TurnBudget { max_turns: 5 }];
        let result = evaluate_criteria(&criteria, "", &no_tools(), 6, 0, 0, None);
        match result {
            CaseResult::Fail(reason) => {
                assert!(reason.contains("turn_budget"), "got: {reason}");
                assert!(reason.contains("5"), "got: {reason}");
                assert!(reason.contains("6"), "got: {reason}");
            }
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    // ── no_error ──────────────────────────────────────────────────────────

    #[test]
    fn eval_no_error_with_no_error_passes() {
        let criteria = vec![Criterion::NoError];
        let result = evaluate_criteria(&criteria, "ok", &no_tools(), 1, 0, 0, None);
        assert!(matches!(result, CaseResult::Pass));
    }

    #[test]
    fn eval_no_error_with_error_fails_with_reason() {
        let criteria = vec![Criterion::NoError];
        let result = evaluate_criteria(
            &criteria,
            "",
            &no_tools(),
            1,
            0,
            0,
            Some("LLM rate limit exceeded"),
        );
        match result {
            CaseResult::Fail(reason) => {
                assert!(reason.contains("no_error"), "got: {reason}");
                assert!(
                    reason.contains("LLM rate limit exceeded"),
                    "got: {reason}"
                );
            }
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    // ── multi-criterion ordering ──────────────────────────────────────────

    #[test]
    fn eval_multi_criteria_first_fail_short_circuits() {
        // First criterion fails → second (tool_called) never evaluated.
        // If it were, tool_called would also fail (no tools called).
        // We verify the reason belongs to the *first* criterion.
        let criteria = vec![
            Criterion::OutputContains {
                pattern: "this_will_not_match".to_string(),
            },
            Criterion::ToolCalled {
                tool: "bash".to_string(),
            },
        ];
        let result = evaluate_criteria(&criteria, "something else", &no_tools(), 1, 0, 0, None);
        match result {
            CaseResult::Fail(reason) => {
                assert!(
                    reason.contains("output_contains"),
                    "first criterion reason expected, got: {reason}"
                );
            }
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    #[test]
    fn eval_empty_criteria_no_eval_passes() {
        // Backward compat: no criteria → Pass (eval handled separately).
        let result = evaluate_criteria(&[], "any output", &no_tools(), 0, 0, 0, None);
        assert!(matches!(result, CaseResult::Pass));
    }

    #[test]
    fn eval_all_criteria_pass_returns_pass() {
        let criteria = vec![
            Criterion::OutputContains {
                pattern: "ok".to_string(),
            },
            Criterion::TurnBudget { max_turns: 3 },
            Criterion::NoError,
        ];
        let result = evaluate_criteria(&criteria, "all ok here", &no_tools(), 2, 0, 0, None);
        assert!(matches!(result, CaseResult::Pass));
    }

    // ── turn budget at exact boundary ─────────────────────────────────────

    #[test]
    fn eval_turn_budget_at_limit_passes() {
        let criteria = vec![Criterion::TurnBudget { max_turns: 5 }];
        let result = evaluate_criteria(&criteria, "", &no_tools(), 5, 0, 0, None);
        assert!(matches!(result, CaseResult::Pass));
    }

    // ── token budget only max_input (no max_output set) ───────────────────

    #[test]
    fn eval_token_budget_only_input_output_ignored() {
        let criteria = vec![Criterion::TokenBudget {
            max_input_tokens: Some(1000),
            max_output_tokens: None,
        }];
        // Even though output_tokens is enormous, no check is applied.
        let result = evaluate_criteria(&criteria, "", &no_tools(), 1, 999, 99999, None);
        assert!(matches!(result, CaseResult::Pass));
    }

    // ════════════════════════════════════════════════════════════════════════
    // case_filter / filter_cases
    // ════════════════════════════════════════════════════════════════════════

    fn make_cases(names: &[&str]) -> Vec<TestCase> {
        names
            .iter()
            .map(|n| TestCase {
                name: n.to_string(),
                message: "hello".to_string(),
                eval: None,
                max_turns: None,
                criteria: vec![],
            })
            .collect()
    }

    #[test]
    fn filter_none_returns_all() {
        let cases = make_cases(&["alpha", "beta", "gamma"]);
        let filtered = filter_cases(&cases, None);
        assert_eq!(filtered.len(), 3);
    }

    #[test]
    fn filter_exact_match_returns_one() {
        let cases = make_cases(&["alpha", "beta", "gamma"]);
        let filtered = filter_cases(&cases, Some("beta"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "beta");
    }

    #[test]
    fn filter_glob_star_matches_multiple() {
        let cases = make_cases(&["test-foo", "test-bar", "other-baz"]);
        let filtered = filter_cases(&cases, Some("test-*"));
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|c| c.name.starts_with("test-")));
    }

    #[test]
    fn filter_glob_question_mark_matches_single_char() {
        let cases = make_cases(&["case-a", "case-b", "case-ab"]);
        let filtered = filter_cases(&cases, Some("case-?"));
        assert_eq!(filtered.len(), 2, "? matches exactly one char");
        assert!(filtered.iter().any(|c| c.name == "case-a"));
        assert!(filtered.iter().any(|c| c.name == "case-b"));
    }

    #[test]
    fn filter_no_match_returns_empty() {
        let cases = make_cases(&["alpha", "beta"]);
        let filtered = filter_cases(&cases, Some("zzz_no_match"));
        assert_eq!(filtered.len(), 0);
    }

    #[test]
    fn filter_is_case_sensitive() {
        let cases = make_cases(&["Alpha", "alpha"]);
        let filtered = filter_cases(&cases, Some("Alpha"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "Alpha");
    }

    // ════════════════════════════════════════════════════════════════════════
    // report_junit / xml_escape
    // ════════════════════════════════════════════════════════════════════════

    fn pass_outcome(name: &str, ms: u64) -> TestOutcome {
        TestOutcome {
            case_name: name.to_string(),
            result: CaseResult::Pass,
            turns: 1,
            duration_ms: ms,
        }
    }

    fn fail_outcome(name: &str, reason: &str, ms: u64) -> TestOutcome {
        TestOutcome {
            case_name: name.to_string(),
            result: CaseResult::Fail(reason.to_string()),
            turns: 1,
            duration_ms: ms,
        }
    }

    fn error_outcome(name: &str, reason: &str, ms: u64) -> TestOutcome {
        TestOutcome {
            case_name: name.to_string(),
            result: CaseResult::Error(reason.to_string()),
            turns: 0,
            duration_ms: ms,
        }
    }

    #[test]
    fn junit_xml_escape_special_chars() {
        assert_eq!(xml_escape("a & b"), "a &amp; b");
        assert_eq!(xml_escape("<tag>"), "&lt;tag&gt;");
        assert_eq!(xml_escape("\"quoted\""), "&quot;quoted&quot;");
        assert_eq!(xml_escape("it's"), "it&apos;s");
    }

    #[test]
    fn junit_empty_outcomes_produces_valid_xml() {
        let dir = tempfile::tempdir().expect("tempdir");
        report_junit("my-suite", &[], dir.path().to_str().unwrap())
            .expect("report_junit");
        let xml_path = dir.path().join("my-suite.xml");
        assert!(xml_path.exists(), "XML file should be created");
        let content = std::fs::read_to_string(&xml_path).unwrap();
        assert!(content.contains("<testsuites"));
        assert!(content.contains("tests=\"0\""));
        assert!(content.contains("failures=\"0\""));
        assert!(content.contains("errors=\"0\""));
        assert!(content.contains("</testsuites>"));
    }

    #[test]
    fn junit_one_pass_produces_testcase_no_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let outcomes = vec![pass_outcome("查询日历", 3200)];
        report_junit("feishu-regression", &outcomes, dir.path().to_str().unwrap())
            .expect("report_junit");
        let xml_path = dir.path().join("feishu-regression.xml");
        let content = std::fs::read_to_string(&xml_path).unwrap();
        assert!(content.contains("查询日历"), "case name should appear");
        assert!(content.contains("<testcase"), "should have testcase element");
        assert!(!content.contains("<failure"), "pass should not have failure");
        assert!(!content.contains("<error"), "pass should not have error");
    }

    #[test]
    fn junit_one_fail_produces_failure_element() {
        let dir = tempfile::tempdir().expect("tempdir");
        let outcomes = vec![fail_outcome("批量更新", "token_budget exceeded: expected max 2000, got 2240", 6100)];
        report_junit("feishu-regression", &outcomes, dir.path().to_str().unwrap())
            .expect("report_junit");
        let xml_path = dir.path().join("feishu-regression.xml");
        let content = std::fs::read_to_string(&xml_path).unwrap();
        assert!(content.contains("<failure"), "fail should have failure element");
        assert!(
            content.contains("token_budget exceeded"),
            "failure message should appear"
        );
        assert!(content.contains("tests=\"1\""));
        assert!(content.contains("failures=\"1\""));
    }

    #[test]
    fn junit_one_error_produces_error_element() {
        let dir = tempfile::tempdir().expect("tempdir");
        let outcomes = vec![error_outcome("连接失败", "failed to build engine: timeout", 500)];
        report_junit("feishu-regression", &outcomes, dir.path().to_str().unwrap())
            .expect("report_junit");
        let xml_path = dir.path().join("feishu-regression.xml");
        let content = std::fs::read_to_string(&xml_path).unwrap();
        assert!(content.contains("<error"), "error should have error element");
        assert!(content.contains("failed to build engine"));
        assert!(content.contains("errors=\"1\""));
    }

    #[test]
    fn junit_output_dir_created_if_missing() {
        let base = tempfile::tempdir().expect("tempdir");
        let nested = base.path().join("a").join("b").join("c");
        // Directory does not exist yet.
        assert!(!nested.exists());
        let outcomes = vec![pass_outcome("test1", 100)];
        report_junit("suite", &outcomes, nested.to_str().unwrap())
            .expect("should create directory and write XML");
        assert!(nested.exists(), "directory should have been created");
        assert!(nested.join("suite.xml").exists());
    }

    #[test]
    fn junit_special_chars_in_case_name_and_failure_escaped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let outcomes = vec![fail_outcome(
            "case <one> & two",
            "error: a < b && c > d",
            100,
        )];
        report_junit("my-suite", &outcomes, dir.path().to_str().unwrap())
            .expect("report_junit");
        let xml_path = dir.path().join("my-suite.xml");
        let content = std::fs::read_to_string(&xml_path).unwrap();
        // Raw < > & must not appear unescaped (except in XML tags themselves)
        // Check that the escaped versions are there.
        assert!(
            content.contains("case &lt;one&gt; &amp; two"),
            "case name should be escaped, got: {content}"
        );
        assert!(
            content.contains("a &lt; b &amp;&amp; c &gt; d"),
            "failure message should be escaped, got: {content}"
        );
    }

    // ════════════════════════════════════════════════════════════════════════
    // case_matches_filter unit tests
    // ════════════════════════════════════════════════════════════════════════

    #[test]
    fn case_matches_exact_name() {
        assert!(case_matches_filter("hello-world", "hello-world"));
    }

    #[test]
    fn case_does_not_match_different_name() {
        assert!(!case_matches_filter("hello-world", "hello"));
    }

    #[test]
    fn case_glob_star_prefix() {
        assert!(case_matches_filter("test-feishu-calendar", "*calendar"));
        assert!(!case_matches_filter("test-feishu-meeting", "*calendar"));
    }

    #[test]
    fn case_glob_star_suffix() {
        assert!(case_matches_filter("feishu-list-events", "feishu-*"));
        assert!(!case_matches_filter("google-list-events", "feishu-*"));
    }

    #[test]
    fn case_glob_question_single_char() {
        assert!(case_matches_filter("case-1", "case-?"));
        assert!(case_matches_filter("case-9", "case-?"));
        assert!(!case_matches_filter("case-10", "case-?"));
    }

    #[test]
    fn case_glob_star_star_matches_all() {
        assert!(case_matches_filter("anything at all", "*"));
        assert!(case_matches_filter("", "*"));
    }

    #[test]
    fn case_filter_is_case_sensitive_match() {
        // Case-sensitive: uppercase does NOT match lowercase filter.
        assert!(!case_matches_filter("Hello", "hello"));
        assert!(case_matches_filter("Hello", "Hello"));
    }
}
