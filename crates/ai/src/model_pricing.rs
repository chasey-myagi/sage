//! Anthropic model pricing table and USD cost calculation.
//!
//! Mirrors CC's `utils/modelCost.ts`. Prices are USD per million tokens.

use crate::types::{Cost, Usage};

struct Pricing {
    input_per_million: f64,
    output_per_million: f64,
    cache_read_per_million: f64,
    cache_write_per_million: f64,
}

impl Pricing {
    const fn new(
        input_per_million: f64,
        output_per_million: f64,
        cache_read_per_million: f64,
        cache_write_per_million: f64,
    ) -> Self {
        Self {
            input_per_million,
            output_per_million,
            cache_read_per_million,
            cache_write_per_million,
        }
    }
}

// Pricing tiers — see https://platform.claude.com/docs/en/about-claude/pricing
const TIER_HAIKU_35: Pricing = Pricing::new(0.8, 4.0, 0.08, 1.0);
const TIER_HAIKU_45: Pricing = Pricing::new(1.0, 5.0, 0.1, 1.25);
const TIER_3_15: Pricing = Pricing::new(3.0, 15.0, 0.3, 3.75);
const TIER_15_75: Pricing = Pricing::new(15.0, 75.0, 1.5, 18.75);
const TIER_5_25: Pricing = Pricing::new(5.0, 25.0, 0.5, 6.25);
// Opus 4.6 fast-mode: $30/$150 per Mtok
const TIER_30_150: Pricing = Pricing::new(30.0, 150.0, 3.0, 37.5);

fn lookup_pricing(model_id: &str) -> &'static Pricing {
    let m = model_id.to_ascii_lowercase();
    if m.contains("haiku-4-5") || m.contains("haiku-4.5") {
        &TIER_HAIKU_45
    } else if m.contains("haiku") {
        &TIER_HAIKU_35
    } else if m.contains("opus-4-6") || m.contains("opus-4.6") {
        // fast mode not yet exposed in this path; use standard tier
        &TIER_5_25
    } else if m.contains("opus-4-5") || m.contains("opus-4.5") {
        &TIER_5_25
    } else if m.contains("opus") {
        &TIER_15_75
    } else {
        // Sonnet 3.5, 3.7, 4, 4.5, 4.6 and unknown models
        &TIER_3_15
    }
}

/// Calculate the USD cost breakdown for `usage` given `model_id`.
///
/// Falls back to the $3/$15 Sonnet tier for unknown models.
pub fn calculate_usd_cost(usage: &Usage, model_id: &str) -> Cost {
    let p = lookup_pricing(model_id);
    let input = (usage.input as f64 / 1_000_000.0) * p.input_per_million;
    let output = (usage.output as f64 / 1_000_000.0) * p.output_per_million;
    let cache_read = (usage.cache_read as f64 / 1_000_000.0) * p.cache_read_per_million;
    let cache_write = (usage.cache_write as f64 / 1_000_000.0) * p.cache_write_per_million;
    let total = input + output + cache_read + cache_write;
    Cost { input, output, cache_read, cache_write, total }
}

/// Like `calculate_usd_cost` but uses Opus 4.6 fast-mode pricing.
pub fn calculate_usd_cost_fast(usage: &Usage) -> Cost {
    let p = &TIER_30_150;
    let input = (usage.input as f64 / 1_000_000.0) * p.input_per_million;
    let output = (usage.output as f64 / 1_000_000.0) * p.output_per_million;
    let cache_read = (usage.cache_read as f64 / 1_000_000.0) * p.cache_read_per_million;
    let cache_write = (usage.cache_write as f64 / 1_000_000.0) * p.cache_write_per_million;
    let total = input + output + cache_read + cache_write;
    Cost { input, output, cache_read, cache_write, total }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_usage(input: u64, output: u64) -> Usage {
        Usage { input, output, ..Usage::default() }
    }

    #[test]
    fn sonnet_basic_cost() {
        // 1M input + 1M output at $3/$15
        let u = make_usage(1_000_000, 1_000_000);
        let cost = calculate_usd_cost(&u, "claude-sonnet-4-5");
        assert!((cost.input - 3.0).abs() < 1e-9);
        assert!((cost.output - 15.0).abs() < 1e-9);
        assert!((cost.total - 18.0).abs() < 1e-9);
    }

    #[test]
    fn haiku_35_cost() {
        let u = make_usage(1_000_000, 0);
        let cost = calculate_usd_cost(&u, "claude-3-5-haiku-20241022");
        assert!((cost.input - 0.8).abs() < 1e-9);
    }

    #[test]
    fn haiku_45_cost() {
        let u = make_usage(1_000_000, 0);
        let cost = calculate_usd_cost(&u, "claude-haiku-4-5");
        assert!((cost.input - 1.0).abs() < 1e-9);
    }

    #[test]
    fn opus_4_cost() {
        let u = make_usage(1_000_000, 0);
        let cost = calculate_usd_cost(&u, "claude-opus-4-20250514");
        assert!((cost.input - 15.0).abs() < 1e-9);
    }

    #[test]
    fn opus_46_standard_cost() {
        let u = make_usage(1_000_000, 0);
        let cost = calculate_usd_cost(&u, "claude-opus-4-6");
        assert!((cost.input - 5.0).abs() < 1e-9);
    }

    #[test]
    fn opus_46_fast_cost() {
        let u = make_usage(1_000_000, 0);
        let cost = calculate_usd_cost_fast(&u);
        assert!((cost.input - 30.0).abs() < 1e-9);
    }

    #[test]
    fn unknown_model_falls_back_to_sonnet_tier() {
        let u = make_usage(1_000_000, 0);
        let cost = calculate_usd_cost(&u, "some-unknown-model");
        assert!((cost.input - 3.0).abs() < 1e-9);
    }

    #[test]
    fn zero_tokens_zero_cost() {
        let u = Usage::default();
        let cost = calculate_usd_cost(&u, "claude-sonnet-4-6");
        assert_eq!(cost.total, 0.0);
    }

    #[test]
    fn cache_tokens_included_in_total() {
        let u = Usage { input: 0, output: 0, cache_read: 1_000_000, cache_write: 0, ..Usage::default() };
        let cost = calculate_usd_cost(&u, "claude-sonnet-4-5");
        assert!((cost.cache_read - 0.3).abs() < 1e-9);
        assert!((cost.total - 0.3).abs() < 1e-9);
    }
}
