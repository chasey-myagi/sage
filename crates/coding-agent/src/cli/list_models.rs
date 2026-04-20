//! List available models with optional fuzzy search.
//!
//! Translated from pi-mono `packages/coding-agent/src/cli/list-models.ts`.

use crate::core::model_resolver::ModelRef;

// ============================================================================
// Formatting helpers
// ============================================================================

/// Format a token count as a human-readable string.
///
/// Mirrors `formatTokenCount()` from TypeScript.
fn format_token_count(count: u64) -> String {
    if count >= 1_000_000 {
        let m = count as f64 / 1_000_000.0;
        if m.fract() == 0.0 {
            format!("{}M", m as u64)
        } else {
            format!("{:.1}M", m)
        }
    } else if count >= 1_000 {
        let k = count as f64 / 1_000.0;
        if k.fract() == 0.0 {
            format!("{}K", k as u64)
        } else {
            format!("{:.1}K", k)
        }
    } else {
        count.to_string()
    }
}

// ============================================================================
// Main entry
// ============================================================================

/// List available models, optionally filtered by search pattern.
///
/// Mirrors `listModels()` from TypeScript. Outputs to stdout in a
/// human-readable aligned table.
pub fn list_models(models: &[ModelRef], search_pattern: Option<&str>) {
    if models.is_empty() {
        println!("No models available. Set API keys in environment variables.");
        return;
    }

    // Filter by search pattern (simple substring match — TS uses fuzzyFilter)
    let filtered: Vec<&ModelRef> = if let Some(pat) = search_pattern {
        let lower = pat.to_lowercase();
        models
            .iter()
            .filter(|m| {
                m.provider.to_lowercase().contains(&lower)
                    || m.id.to_lowercase().contains(&lower)
                    || m.name.as_ref().map_or(false, |n| n.to_lowercase().contains(&lower))
            })
            .collect()
    } else {
        models.iter().collect()
    };

    if filtered.is_empty() {
        println!(
            "No models matching \"{}\"",
            search_pattern.unwrap_or("")
        );
        return;
    }

    // Sort: provider asc, then model id asc
    let mut sorted = filtered;
    sorted.sort_by(|a, b| {
        let cmp = a.provider.cmp(&b.provider);
        if cmp != std::cmp::Ordering::Equal {
            cmp
        } else {
            a.id.cmp(&b.id)
        }
    });

    // Build rows
    struct Row {
        provider: String,
        model: String,
        context: String,
        max_out: String,
        thinking: String,
    }

    let rows: Vec<Row> = sorted
        .iter()
        .map(|m| Row {
            provider: m.provider.clone(),
            model: m.id.clone(),
            context: format_token_count(m.context_window),
            max_out: format_token_count(m.max_tokens),
            thinking: if m.reasoning { "yes" } else { "no" }.to_string(),
        })
        .collect();

    let h_provider = "provider";
    let h_model = "model";
    let h_context = "context";
    let h_max_out = "max-out";
    let h_thinking = "thinking";

    let w_provider = rows.iter().map(|r| r.provider.len()).max().unwrap_or(0).max(h_provider.len());
    let w_model = rows.iter().map(|r| r.model.len()).max().unwrap_or(0).max(h_model.len());
    let w_context = rows.iter().map(|r| r.context.len()).max().unwrap_or(0).max(h_context.len());
    let w_max_out = rows.iter().map(|r| r.max_out.len()).max().unwrap_or(0).max(h_max_out.len());
    let w_thinking = rows.iter().map(|r| r.thinking.len()).max().unwrap_or(0).max(h_thinking.len());

    let header = format!(
        "{:<wp$}  {:<wm$}  {:<wc$}  {:<wo$}  {:<wt$}",
        h_provider,
        h_model,
        h_context,
        h_max_out,
        h_thinking,
        wp = w_provider,
        wm = w_model,
        wc = w_context,
        wo = w_max_out,
        wt = w_thinking,
    );
    println!("{header}");

    for row in &rows {
        let line = format!(
            "{:<wp$}  {:<wm$}  {:<wc$}  {:<wo$}  {:<wt$}",
            row.provider,
            row.model,
            row.context,
            row.max_out,
            row.thinking,
            wp = w_provider,
            wm = w_model,
            wc = w_context,
            wo = w_max_out,
            wt = w_thinking,
        );
        println!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_token_count_millions() {
        assert_eq!(format_token_count(1_000_000), "1M");
        assert_eq!(format_token_count(1_500_000), "1.5M");
    }

    #[test]
    fn format_token_count_thousands() {
        assert_eq!(format_token_count(128_000), "128K");
        assert_eq!(format_token_count(1_500), "1.5K");
    }

    #[test]
    fn format_token_count_small() {
        assert_eq!(format_token_count(512), "512");
    }

    #[test]
    fn list_models_no_models() {
        // Just verify it doesn't panic
        list_models(&[], None);
    }
}
