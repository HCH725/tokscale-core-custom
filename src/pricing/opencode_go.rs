//! Official OpenCode Go quota-value pricing for Hermes/OpenCode Go traffic.
//!
//! Source of truth: https://opencode.ai/docs/go/
//! Rates are USD per 1M tokens and represent the value OpenCode Go uses toward
//! its rolling / weekly / monthly usage limits. DeepSeek V4 uses documented
//! weekday UTC peak windows; all other times, including weekends, are off-peak.

use chrono::{Datelike, Timelike, Utc, Weekday};

use crate::TokenBreakdown;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Rates {
    input_per_million: f64,
    output_per_million: f64,
    cache_read_per_million: f64,
    cache_write_per_million: Option<f64>,
}

impl Rates {
    fn estimate(self, usage: &TokenBreakdown) -> f64 {
        const MILLION: f64 = 1_000_000.0;
        let input = usage.input.max(0) as f64 * self.input_per_million / MILLION;
        let output = usage.output.max(0) as f64 * self.output_per_million / MILLION;
        let cache_read = usage.cache_read.max(0) as f64 * self.cache_read_per_million / MILLION;
        let cache_write = self
            .cache_write_per_million
            .map_or(0.0, |rate| usage.cache_write.max(0) as f64 * rate / MILLION);
        input + output + cache_read + cache_write
    }
}

fn normalize_model_id(model_id: &str) -> String {
    let lower = model_id.trim().to_ascii_lowercase();
    lower
        .strip_prefix("opencode-go/")
        .or_else(|| lower.strip_prefix("opencode_go/"))
        .unwrap_or(&lower)
        .to_string()
}

fn is_deepseek_peak_utc(timestamp_ms: i64) -> bool {
    let Some(dt) = chrono::DateTime::<Utc>::from_timestamp_millis(timestamp_ms) else {
        return false;
    };
    if matches!(dt.weekday(), Weekday::Sat | Weekday::Sun) {
        return false;
    }
    let hour = dt.hour();
    (1..4).contains(&hour) || (6..10).contains(&hour)
}

fn rates_for(model_id: &str, timestamp_ms: i64) -> Option<Rates> {
    let model = normalize_model_id(model_id);
    let rates = match model.as_str() {
        "mimo-v2.5" => Rates {
            input_per_million: 0.14,
            output_per_million: 0.28,
            cache_read_per_million: 0.0028,
            cache_write_per_million: None,
        },
        "mimo-v2.5-pro" => Rates {
            input_per_million: 0.435,
            output_per_million: 0.87,
            cache_read_per_million: 0.003625,
            cache_write_per_million: None,
        },
        "muse-spark-1.2-contributor" => Rates {
            input_per_million: 0.10,
            output_per_million: 0.20,
            cache_read_per_million: 0.002,
            cache_write_per_million: None,
        },
        "deepseek-v4-pro" => {
            if is_deepseek_peak_utc(timestamp_ms) {
                Rates {
                    input_per_million: 1.32,
                    output_per_million: 3.96,
                    cache_read_per_million: 0.044,
                    cache_write_per_million: None,
                }
            } else {
                Rates {
                    input_per_million: 0.66,
                    output_per_million: 1.98,
                    cache_read_per_million: 0.022,
                    cache_write_per_million: None,
                }
            }
        }
        "deepseek-v4-flash" | "deepseek-v4-flash-vision-exp" => {
            if is_deepseek_peak_utc(timestamp_ms) {
                Rates {
                    input_per_million: 0.44,
                    output_per_million: 1.32,
                    cache_read_per_million: 0.014,
                    cache_write_per_million: None,
                }
            } else {
                Rates {
                    input_per_million: 0.22,
                    output_per_million: 0.66,
                    cache_read_per_million: 0.007,
                    cache_write_per_million: None,
                }
            }
        }
        _ => return None,
    };
    Some(rates)
}

pub(crate) fn estimate_cost(
    model_id: &str,
    timestamp_ms: i64,
    usage: &TokenBreakdown,
) -> Option<f64> {
    rates_for(model_id, timestamp_ms).map(|rates| rates.estimate(usage))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn usage() -> TokenBreakdown {
        TokenBreakdown {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 1_000_000,
            cache_write: 0,
            cache_write_1h: 0,
            reasoning: 0,
        }
    }

    fn ts(year: i32, month: u32, day: u32, hour: u32) -> i64 {
        Utc.with_ymd_and_hms(year, month, day, hour, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis()
    }

    #[test]
    fn mimo_v25_uses_official_go_rates() {
        let cost = estimate_cost("mimo-v2.5", ts(2026, 9, 1, 12), &usage()).unwrap();
        assert!((cost - (0.14 + 0.28 + 0.0028)).abs() < 1e-12);
    }

    #[test]
    fn muse_contributor_uses_official_go_rates() {
        let cost = estimate_cost(
            "opencode-go/muse-spark-1.2-contributor",
            ts(2026, 9, 1, 12),
            &usage(),
        )
        .unwrap();
        assert!((cost - (0.10 + 0.20 + 0.002)).abs() < 1e-12);
    }

    #[test]
    fn deepseek_flash_switches_between_peak_and_off_peak() {
        // 2026-09-01 is Tuesday. 02:00 UTC is peak; 05:00 UTC is off-peak.
        let peak = estimate_cost("deepseek-v4-flash", ts(2026, 9, 1, 2), &usage()).unwrap();
        let off_peak = estimate_cost("deepseek-v4-flash", ts(2026, 9, 1, 5), &usage()).unwrap();
        assert!((peak - (0.44 + 1.32 + 0.014)).abs() < 1e-12);
        assert!((off_peak - (0.22 + 0.66 + 0.007)).abs() < 1e-12);
    }

    #[test]
    fn deepseek_weekend_is_always_off_peak() {
        // 2026-09-05 is Saturday; 02:00 UTC would be peak on a weekday.
        let cost = estimate_cost("deepseek-v4-pro", ts(2026, 9, 5, 2), &usage()).unwrap();
        assert!((cost - (0.66 + 1.98 + 0.022)).abs() < 1e-12);
    }

    #[test]
    fn unknown_go_model_falls_back_to_general_pricing() {
        assert!(estimate_cost("glm-5.3", ts(2026, 9, 1, 12), &usage()).is_none());
    }
}
