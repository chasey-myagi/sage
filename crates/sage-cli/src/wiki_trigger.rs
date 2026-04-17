// Sprint 7 — S7.2: Pure decision logic for triggering a `WikiMaintenance` run.
//
// The daemon reuses this function every time a client disconnects; it's the
// narrowest possible unit so we can test the branching without spinning up
// a real SageSession.

use sage_runner::config::WikiConfig;

/// Decide whether the daemon should kick off a `WikiMaintenance` session now.
///
/// - `config`              — agent's wiki config (may be disabled or unset)
/// - `unprocessed_count`   — number of archived sessions not yet mentioned
///                           in `wiki/log.md`
/// - `last_maintenance_ts` — unix seconds of the previous run (`None` if
///                           we've never run maintenance on this workspace)
/// - `now_ts`              — unix seconds "now" (injected so tests are
///                           deterministic)
///
/// Returns `true` only if **all** of:
/// 1. `config.enabled` is true,
/// 2. `unprocessed_count >= config.trigger_sessions` (and > 0 — triggering
///    with nothing to process is pointless),
/// 3. The cooldown has elapsed: either we've never run, or
///    `now_ts - last_maintenance_ts >= config.cooldown_secs`.
pub fn should_trigger_wiki_maintenance(
    config: &WikiConfig,
    unprocessed_count: usize,
    last_maintenance_ts: Option<u64>,
    now_unix_secs: u64,
) -> bool {
    if !config.enabled {
        return false;
    }

    if unprocessed_count == 0 {
        return false;
    }

    if (unprocessed_count as u64) < (config.trigger_sessions as u64) {
        return false;
    }

    if let Some(last) = last_maintenance_ts {
        // Clock went backwards — treat as "cooldown not yet elapsed".
        // Saturating_sub would produce 0 and incorrectly trigger.
        if last > now_unix_secs {
            return false;
        }
        let elapsed = now_unix_secs - last;
        if elapsed < config.cooldown_secs {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_cfg(trigger: u32, cooldown: u64) -> WikiConfig {
        WikiConfig {
            trigger_sessions: trigger,
            cooldown_secs: cooldown,
            enabled: true,
        }
    }

    #[test]
    fn disabled_never_triggers_even_when_threshold_met() {
        let cfg = WikiConfig {
            trigger_sessions: 3,
            cooldown_secs: 1800,
            enabled: false,
        };
        // Overwhelmingly satisfies every other condition.
        assert!(!should_trigger_wiki_maintenance(&cfg, 999, None, 10_000));
        assert!(!should_trigger_wiki_maintenance(&cfg, 3, Some(0), 10_000));
    }

    #[test]
    fn unprocessed_below_threshold_does_not_trigger() {
        let cfg = enabled_cfg(3, 1800);
        assert!(!should_trigger_wiki_maintenance(&cfg, 2, None, 10_000));
        assert!(!should_trigger_wiki_maintenance(&cfg, 0, None, 10_000));
    }

    #[test]
    fn threshold_met_but_cooldown_active_does_not_trigger() {
        let cfg = enabled_cfg(3, 1800);
        let last = 10_000u64;
        let now = last + 900; // 900s < 1800s cooldown
        assert!(!should_trigger_wiki_maintenance(&cfg, 3, Some(last), now));
    }

    #[test]
    fn threshold_met_and_cooldown_elapsed_triggers() {
        let cfg = enabled_cfg(3, 1800);
        let last = 10_000u64;
        let now = last + 1800; // exactly at the boundary
        assert!(should_trigger_wiki_maintenance(&cfg, 3, Some(last), now));

        let now2 = last + 10_000; // well past
        assert!(should_trigger_wiki_maintenance(&cfg, 5, Some(last), now2));
    }

    #[test]
    fn never_maintained_and_threshold_met_triggers() {
        let cfg = enabled_cfg(3, 1800);
        assert!(should_trigger_wiki_maintenance(&cfg, 3, None, 0));
        assert!(should_trigger_wiki_maintenance(&cfg, 100, None, 123_456));
    }

    #[test]
    fn enabled_with_zero_unprocessed_does_not_trigger() {
        let cfg = enabled_cfg(3, 1800);
        // Even if the last-maintenance bookkeeping says we could, we must not
        // spin up an empty maintenance session.
        assert!(!should_trigger_wiki_maintenance(&cfg, 0, None, 99_999));
        assert!(!should_trigger_wiki_maintenance(&cfg, 0, Some(0), 99_999));
    }

    #[test]
    fn threshold_of_one_triggers_on_single_unprocessed() {
        let cfg = enabled_cfg(1, 60);
        assert!(should_trigger_wiki_maintenance(&cfg, 1, None, 100));
    }
}
