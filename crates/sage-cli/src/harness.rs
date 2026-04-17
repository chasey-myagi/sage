// Harness runner — evaluates agent outputs by running test suites with eval scripts.

use anyhow::{Context as _, Result};
use sage_runtime::event::{AgentEvent, AgentEventSink};
use sage_runtime::types::AgentMessage;
use serde::{Deserialize, Serialize};
use std::time::Instant;
use tokio::sync::Mutex;

// ── Test suite YAML schema ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TestCase {
    pub name: String,
    pub message: String,
    /// Optional path to eval script. If absent, always Pass.
    pub eval: Option<String>,
    /// Optional per-case max turns override (currently advisory; future use).
    pub max_turns: Option<usize>,
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

// ── run_test_suite ────────────────────────────────────────────────────────────

/// Load and run a test suite YAML file.
///
/// Returns `true` if all cases passed, `false` if any failed or errored.
pub async fn run_test_suite(suite_path: &str, reporter: Reporter) -> Result<bool> {
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

    let mut outcomes: Vec<TestOutcome> = Vec::new();

    for case in &suite.cases {
        tracing::info!(case = %case.name, "running test case");
        let start = Instant::now();

        // Build a fresh engine for each case (dev mode = true, skips VM)
        let engine = match crate::serve::build_engine_for_agent(&config, true).await {
            Ok(e) => e,
            Err(e) => {
                let duration_ms = start.elapsed().as_millis() as u64;
                outcomes.push(TestOutcome {
                    case_name: case.name.clone(),
                    result: CaseResult::Error(format!("failed to build engine: {e}")),
                    turns: 0,
                    duration_ms,
                });
                continue;
            }
        };

        // Create session
        let mut session = match engine.session().await {
            Ok(s) => s,
            Err(e) => {
                let duration_ms = start.elapsed().as_millis() as u64;
                outcomes.push(TestOutcome {
                    case_name: case.name.clone(),
                    result: CaseResult::Error(format!("failed to create session: {e}")),
                    turns: 0,
                    duration_ms,
                });
                continue;
            }
        };

        // Send test message
        let sink = TurnCountingSink::new();
        let send_result = session.send(&case.message, &sink).await;
        let turns = sink.turn_count().await;
        let duration_ms = start.elapsed().as_millis() as u64;

        if let Err(e) = send_result {
            outcomes.push(TestOutcome {
                case_name: case.name.clone(),
                result: CaseResult::Error(format!("agent error: {e}")),
                turns,
                duration_ms,
            });
            continue;
        }

        // Extract last assistant response
        let last_text = last_assistant_text(session.messages());

        // Evaluate
        let result = match &case.eval {
            None => CaseResult::Pass,
            Some(script) => {
                run_eval_script(script, &last_text, &suite.agent, &case.name).await
            }
        };

        outcomes.push(TestOutcome {
            case_name: case.name.clone(),
            result,
            turns,
            duration_ms,
        });
    }

    // Report
    match reporter {
        Reporter::Terminal => report_terminal(&outcomes),
        Reporter::Json => report_json(&outcomes),
    }

    // Return true only if all outcomes are Pass
    let all_pass = outcomes.iter().all(|o| matches!(o.result, CaseResult::Pass));
    Ok(all_pass)
}
