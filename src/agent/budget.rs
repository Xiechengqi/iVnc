//! Token-cost estimation for agent provider usage.
//!
//! Per-token prices are tiny in absolute terms, so the table is keyed by
//! *micros-USD per 1,000,000 tokens* — i.e. the published per-million price
//! converted to micros. For a given `ProviderUsage`, total cost is
//! `tokens * micros_per_million / 1_000_000`, which keeps everything in
//! integer arithmetic.

use super::types::ProviderUsage;

#[derive(Debug, Clone, Copy)]
struct Price {
    input_micros_per_million: u64,
    output_micros_per_million: u64,
}

/// Compute aggregate cost in USD micros for a single ProviderUsage.
/// Returns None when we have no published price for the (provider, model).
pub fn estimate_cost_usd_micros(
    provider: &str,
    model: &str,
    usage: &ProviderUsage,
) -> Option<u64> {
    let price = price_for(provider, model)?;
    let input_cost = usage
        .input_tokens
        .saturating_mul(price.input_micros_per_million)
        / 1_000_000;
    let output_cost = usage
        .output_tokens
        .saturating_mul(price.output_micros_per_million)
        / 1_000_000;
    Some(input_cost.saturating_add(output_cost))
}

fn price_for(provider: &str, model: &str) -> Option<Price> {
    match provider {
        "replay" | "local" | "holo3" => return None,
        _ => {}
    }
    let m = model.to_ascii_lowercase();
    let p = match m.as_str() {
        "gpt-4o" | "gpt-4o-2024-08-06" | "gpt-4o-2024-11-20" => Price {
            input_micros_per_million: 5_000_000,
            output_micros_per_million: 15_000_000,
        },
        "gpt-4o-mini" => Price {
            input_micros_per_million: 150_000,
            output_micros_per_million: 600_000,
        },
        "gpt-4.1" | "gpt-4.1-2025-04-14" => Price {
            input_micros_per_million: 2_500_000,
            output_micros_per_million: 10_000_000,
        },
        "gpt-4.1-mini" => Price {
            input_micros_per_million: 500_000,
            output_micros_per_million: 2_000_000,
        },
        "gpt-4.1-nano" => Price {
            input_micros_per_million: 100_000,
            output_micros_per_million: 400_000,
        },
        "claude-opus-4" | "claude-opus-4-5" | "claude-opus-4-7" => Price {
            input_micros_per_million: 15_000_000,
            output_micros_per_million: 75_000_000,
        },
        "claude-sonnet-4" | "claude-sonnet-4-5" | "claude-sonnet-4-6" => Price {
            input_micros_per_million: 3_000_000,
            output_micros_per_million: 15_000_000,
        },
        "claude-haiku-4-5" | "claude-haiku-4-5-20251001" => Price {
            input_micros_per_million: 1_000_000,
            output_micros_per_million: 5_000_000,
        },
        "gemini-2.0-flash" | "gemini-2.0-flash-001" => Price {
            input_micros_per_million: 100_000,
            output_micros_per_million: 400_000,
        },
        "gemini-2.5-flash" => Price {
            input_micros_per_million: 300_000,
            output_micros_per_million: 2_500_000,
        },
        "gemini-2.5-pro" => Price {
            input_micros_per_million: 1_250_000,
            output_micros_per_million: 10_000_000,
        },
        "gemini-1.5-flash" => Price {
            input_micros_per_million: 75_000,
            output_micros_per_million: 300_000,
        },
        "gemini-1.5-pro" => Price {
            input_micros_per_million: 1_250_000,
            output_micros_per_million: 5_000_000,
        },
        _ => return None,
    };
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: u64, output: u64) -> ProviderUsage {
        ProviderUsage {
            input_tokens: input,
            output_tokens: output,
            provider_latency_ms: 0,
            cost_usd_micros: None,
        }
    }

    #[test]
    fn computes_gpt4o_cost_in_micros() {
        // 1M input + 1M output at $5/$15 → $20 = 20_000_000 micros.
        let cost =
            estimate_cost_usd_micros("openai", "gpt-4o", &usage(1_000_000, 1_000_000)).unwrap();
        assert_eq!(cost, 20_000_000);
    }

    #[test]
    fn small_usage_truncates_to_micros() {
        // 1 input + 1 output tokens on gpt-4o-mini → 0.15 + 0.6 micros, integer truncates to 0.
        let cost = estimate_cost_usd_micros("openai", "gpt-4o-mini", &usage(1, 1)).unwrap();
        assert_eq!(cost, 0);
        // 10 input tokens → 1.5 micros → 1 (floor).
        let cost = estimate_cost_usd_micros("openai", "gpt-4o-mini", &usage(10, 0)).unwrap();
        assert_eq!(cost, 1);
        // 1000/1000 on gpt-4o-mini → 150 + 600 = 750 micros total.
        let cost = estimate_cost_usd_micros("openai", "gpt-4o-mini", &usage(1000, 1000)).unwrap();
        assert_eq!(cost, 750);
    }

    #[test]
    fn free_providers_return_none() {
        assert!(estimate_cost_usd_micros("replay", "any", &usage(1_000_000, 1_000_000)).is_none());
        assert!(estimate_cost_usd_micros("local", "any", &usage(1_000_000, 1_000_000)).is_none());
        assert!(estimate_cost_usd_micros("holo3", "any", &usage(1_000_000, 1_000_000)).is_none());
    }

    #[test]
    fn unknown_model_returns_none() {
        assert!(
            estimate_cost_usd_micros("openai", "some-future-model", &usage(1000, 1000)).is_none()
        );
    }
}
