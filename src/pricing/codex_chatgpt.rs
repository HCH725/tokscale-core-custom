//! ChatGPT Work / Codex token-rate equivalent for subscription usage.
//!
//! Source of truth: https://help.openai.com/en/articles/20001415
//! Rates are USD per 1M tokens for supported Work/Codex activity. They are a
//! list-price equivalent only; ChatGPT plan 5-hour/weekly allowance depletion
//! is provider-reported separately and must not be inferred from these dollars.

use crate::TokenBreakdown;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Rates {
    input_per_million: f64,
    cached_input_per_million: f64,
    output_per_million: f64,
}

impl Rates {
    fn estimate(self, usage: &TokenBreakdown) -> f64 {
        const MILLION: f64 = 1_000_000.0;
        usage.input.max(0) as f64 * self.input_per_million / MILLION
            + usage.cache_read.max(0) as f64 * self.cached_input_per_million / MILLION
            + usage.output.max(0) as f64 * self.output_per_million / MILLION
    }
}

fn normalize_model_id(model_id: &str) -> String {
    let lower = model_id.trim().to_ascii_lowercase();
    lower.rsplit('/').next().unwrap_or(&lower).replace('_', "-")
}

fn rates_for(model_id: &str) -> Option<Rates> {
    let model = normalize_model_id(model_id);
    let rates = match model.as_str() {
        "gpt-5.6-sol" => Rates {
            input_per_million: 4.0,
            cached_input_per_million: 0.40,
            output_per_million: 20.0,
        },
        "gpt-5.6-terra" => Rates {
            input_per_million: 2.0,
            cached_input_per_million: 0.20,
            output_per_million: 12.0,
        },
        "gpt-5.6-luna" => Rates {
            input_per_million: 0.20,
            cached_input_per_million: 0.02,
            output_per_million: 1.20,
        },
        "gpt-5.5" => Rates {
            input_per_million: 5.0,
            cached_input_per_million: 0.50,
            output_per_million: 30.0,
        },
        "gpt-5.4" => Rates {
            input_per_million: 2.50,
            cached_input_per_million: 0.25,
            output_per_million: 15.0,
        },
        "gpt-5.4-mini" => Rates {
            input_per_million: 0.75,
            cached_input_per_million: 0.075,
            output_per_million: 4.50,
        },
        "gpt-5.3-codex" | "gpt-5.2" => Rates {
            input_per_million: 1.75,
            cached_input_per_million: 0.175,
            output_per_million: 14.0,
        },
        _ => return None,
    };
    Some(rates)
}

/// Estimate the recorded model's Work/Codex list-price equivalent.
///
/// `reasoning` is deliberately not added. Hermes' accounting `output_tokens`
/// already includes reasoning tokens for Codex subscription sessions, so adding
/// the separate diagnostic reasoning bucket here would double-charge output.
pub(crate) fn estimate_cost(model_id: &str, usage: &TokenBreakdown) -> Option<f64> {
    rates_for(model_id).map(|rates| rates.estimate(usage))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage() -> TokenBreakdown {
        TokenBreakdown {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 1_000_000,
            cache_write: 1_000_000,
            cache_write_1h: 0,
            reasoning: 1_000_000,
        }
    }

    #[test]
    fn terra_uses_official_work_codex_rates() {
        let cost = estimate_cost("gpt-5.6-terra", &usage()).unwrap();
        assert!((cost - 14.20).abs() < 1e-12);
    }

    #[test]
    fn recorded_gpt54_uses_recorded_model_rate() {
        let cost = estimate_cost("openai/gpt-5.4", &usage()).unwrap();
        assert!((cost - 17.75).abs() < 1e-12);
    }

    #[test]
    fn cache_write_and_reasoning_are_not_double_charged() {
        let only_unpriced_buckets = TokenBreakdown {
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 1_000_000,
            cache_write_1h: 0,
            reasoning: 1_000_000,
        };
        assert_eq!(
            estimate_cost("gpt-5.6-terra", &only_unpriced_buckets),
            Some(0.0)
        );
    }

    #[test]
    fn unknown_model_is_not_guessed() {
        assert_eq!(estimate_cost("gpt-future", &usage()), None);
    }
}
