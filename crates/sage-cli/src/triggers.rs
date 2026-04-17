// Triggers — cron-based scheduled message dispatch to agent daemons.
//
// Config: ~/.sage/triggers.yaml
//
// Example:
//   triggers:
//     - name: morning-briefing
//       cron: "0 9 * * *"      # 09:00 daily
//       agent: feishu
//       message: "请汇总今日日程安排"
//       enabled: true
//
//     - name: hourly-check
//       every_secs: 3600
//       agent: coder
//       message: "check for any pending tasks"
//       enabled: true

use anyhow::{Context as _, Result};
use chrono::{Datelike, Local, Timelike};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use tokio::{
    io::{AsyncBufReadExt as _, AsyncWriteExt as _},
    net::UnixStream,
};

// ── Config types ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
struct TriggersFile {
    #[serde(default)]
    triggers: Vec<TriggerConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TriggerConfig {
    pub name: String,
    #[serde(default)]
    pub cron: Option<String>,
    #[serde(default)]
    pub every_secs: Option<u64>,
    pub agent: String,
    pub message: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

// ── File location ─────────────────────────────────────────────────────

fn triggers_path() -> Result<PathBuf> {
    let home = sage_runner::home_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;
    Ok(home.join(".sage").join("triggers.yaml"))
}

fn socket_path(agent: &str) -> PathBuf {
    PathBuf::from(format!("/tmp/sage-{agent}.sock"))
}

// ── Config loading ────────────────────────────────────────────────────

async fn load_triggers() -> Result<Vec<TriggerConfig>> {
    let path = triggers_path()?;
    let content = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("cannot read triggers config at {}", path.display()))?;
    let file: TriggersFile = serde_yaml::from_str(&content)
        .with_context(|| format!("invalid YAML in {}", path.display()))?;
    Ok(file.triggers)
}

// ── Minimal 5-field cron parser ───────────────────────────────────────
//
// Fields: minute hour dom month dow
//   *         matches any value
//   N         matches exact value
//   N-M       matches inclusive range
//   */N       matches every N steps from 0
//   a,b,c     matches any listed value

fn cron_field_matches(field: &str, value: u32) -> bool {
    for part in field.split(',') {
        if part == "*" {
            return true;
        }
        if let Some(step_str) = part.strip_prefix("*/") {
            if let Ok(step) = step_str.parse::<u32>() {
                if step > 0 && value % step == 0 {
                    return true;
                }
            }
            continue;
        }
        if let Some((lo, hi)) = part.split_once('-') {
            if let (Ok(lo), Ok(hi)) = (lo.parse::<u32>(), hi.parse::<u32>()) {
                if value >= lo && value <= hi {
                    return true;
                }
            }
            continue;
        }
        if let Ok(n) = part.parse::<u32>() {
            if n == value {
                return true;
            }
        }
    }
    false
}

/// Returns true if the trigger's cron expression matches `now` (checked at minute granularity).
fn cron_matches(cron: &str, now: &chrono::DateTime<Local>) -> bool {
    let fields: Vec<&str> = cron.split_whitespace().collect();
    if fields.len() < 5 {
        return false;
    }
    let minute = now.minute();
    let hour = now.hour();
    let dom = now.day();
    let month = now.month();
    let dow = now.weekday().num_days_from_sunday(); // 0=Sun…6=Sat

    cron_field_matches(fields[0], minute)
        && cron_field_matches(fields[1], hour)
        && cron_field_matches(fields[2], dom)
        && cron_field_matches(fields[3], month)
        && cron_field_matches(fields[4], dow)
}

// ── Fire a trigger ────────────────────────────────────────────────────

async fn fire(trigger: &TriggerConfig) {
    let sock = socket_path(&trigger.agent);
    let stream = match UnixStream::connect(&sock).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(
                trigger = %trigger.name,
                agent = %trigger.agent,
                err = %e,
                "trigger: cannot connect to agent socket"
            );
            return;
        }
    };

    let (read_half, mut write_half) = stream.into_split();

    let msg = serde_json::json!({ "type": "send", "text": trigger.message });
    let line = msg.to_string() + "\n";
    if let Err(e) = write_half.write_all(line.as_bytes()).await {
        tracing::warn!(trigger = %trigger.name, err = %e, "trigger: write failed");
        return;
    }

    // Drain response until RunEnd / RunError
    let mut reader = tokio::io::BufReader::new(read_half);
    let mut srv_line = String::new();
    loop {
        srv_line.clear();
        match reader.read_line(&mut srv_line).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let trimmed = srv_line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
                    match v.get("type").and_then(|t| t.as_str()) {
                        Some("run_end") | Some("run_error") => break,
                        _ => {}
                    }
                }
            }
        }
    }

    tracing::info!(trigger = %trigger.name, agent = %trigger.agent, "trigger fired");
}

// ── Public: list triggers ─────────────────────────────────────────────

pub async fn list_triggers() -> Result<()> {
    let triggers = load_triggers().await?;

    if triggers.is_empty() {
        println!("No triggers configured.");
        println!("Edit ~/.sage/triggers.yaml to add triggers.");
        return Ok(());
    }

    println!("{:<24} {:<16} {:<20} {}", "NAME", "AGENT", "SCHEDULE", "ENABLED");
    println!("{}", "─".repeat(70));
    for t in &triggers {
        let schedule = if let Some(c) = &t.cron {
            c.clone()
        } else if let Some(s) = t.every_secs {
            format!("every {s}s")
        } else {
            "(none)".into()
        };
        println!(
            "{:<24} {:<16} {:<20} {}",
            t.name,
            t.agent,
            schedule,
            if t.enabled { "yes" } else { "no" }
        );
    }

    Ok(())
}

// ── Public: run trigger loop ──────────────────────────────────────────

/// Start the trigger scheduler. Runs indefinitely.
///
/// Cron triggers are evaluated once per minute on the minute boundary.
/// Interval triggers fire every N seconds.
pub async fn run_triggers() -> Result<()> {
    let triggers = load_triggers().await?;

    if triggers.is_empty() {
        println!("No triggers configured in ~/.sage/triggers.yaml");
        return Ok(());
    }

    let enabled: Vec<TriggerConfig> = triggers.into_iter().filter(|t| t.enabled).collect();
    if enabled.is_empty() {
        println!("All triggers are disabled.");
        return Ok(());
    }

    println!("Starting trigger scheduler ({} active trigger(s))…", enabled.len());

    // Spawn interval-based triggers as independent tasks
    let mut handles = Vec::new();
    for trigger in &enabled {
        if let Some(secs) = trigger.every_secs {
            let t = trigger.clone();
            let handle = tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(secs));
                interval.tick().await; // skip first immediate tick
                loop {
                    interval.tick().await;
                    fire(&t).await;
                }
            });
            handles.push(handle);
        }
    }

    // Cron triggers: tick every minute on the 0-second boundary
    let cron_triggers: Vec<TriggerConfig> = enabled
        .iter()
        .filter(|t| t.cron.is_some())
        .cloned()
        .collect();

    if !cron_triggers.is_empty() {
        let handle = tokio::spawn(async move {
            loop {
                // Sleep until the next whole minute
                let now = Local::now();
                let secs_into_minute = now.second();
                let sleep_secs = 60 - secs_into_minute as u64;
                tokio::time::sleep(std::time::Duration::from_secs(sleep_secs)).await;

                let now = Local::now();
                for t in &cron_triggers {
                    if let Some(expr) = &t.cron {
                        if cron_matches(expr, &now) {
                            fire(t).await;
                        }
                    }
                }
            }
        });
        handles.push(handle);
    }

    // Wait for all tasks (they loop forever — Ctrl+C terminates)
    for h in handles {
        let _ = h.await;
    }

    Ok(())
}

// ── Unit tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::cron_field_matches;

    #[test]
    fn wildcard_matches_any() {
        assert!(cron_field_matches("*", 0));
        assert!(cron_field_matches("*", 59));
    }

    #[test]
    fn exact_matches() {
        assert!(cron_field_matches("9", 9));
        assert!(!cron_field_matches("9", 10));
    }

    #[test]
    fn range_matches() {
        assert!(cron_field_matches("8-10", 8));
        assert!(cron_field_matches("8-10", 10));
        assert!(!cron_field_matches("8-10", 7));
        assert!(!cron_field_matches("8-10", 11));
    }

    #[test]
    fn step_matches() {
        // */15 → 0, 15, 30, 45
        assert!(cron_field_matches("*/15", 0));
        assert!(cron_field_matches("*/15", 15));
        assert!(!cron_field_matches("*/15", 1));
    }

    #[test]
    fn list_matches() {
        assert!(cron_field_matches("1,15,30", 15));
        assert!(!cron_field_matches("1,15,30", 2));
    }
}
