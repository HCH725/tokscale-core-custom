//! Hermes Agent session parser
//!
//! Parses aggregated session rows from Hermes Agent's SQLite state database:
//! - `~/.hermes/state.db`
//! - `~/.hermes/profiles/<profile>/state.db`
//! - `$HERMES_HOME/state.db`

use super::UnifiedMessage;
use crate::{provider_identity, TokenBreakdown};
use rusqlite::Connection;
use serde::Deserialize;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use tracing::warn;

const HERMES_AGENT_NAME: &str = "Hermes Agent";
const CATDESK_MODEL_ID: &str = "catdesk-mcp";
const CATDESK_PROVIDER_ID: &str = "catdesk";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatDeskUsageEntry {
    timestamp_ms: u64,
    input_tokens: u64,
    output_tokens: u64,
    #[serde(rename = "bucket")]
    _bucket: String,
}

fn timestamp_secs_to_ms(timestamp: f64) -> i64 {
    if timestamp > 1e12 {
        timestamp as i64
    } else {
        (timestamp * 1000.0) as i64
    }
}

fn resolved_provider(billing_provider: Option<String>, model_id: &str) -> String {
    billing_provider
        .filter(|provider| !provider.trim().is_empty())
        .and_then(|provider| provider_identity::canonical_provider(provider.trim()))
        .or_else(|| provider_identity::inferred_provider_from_model(model_id).map(str::to_string))
        .unwrap_or_else(|| "hermes".to_string())
}

fn valid_cost(cost: Option<f64>) -> Option<f64> {
    cost.filter(|cost| cost.is_finite() && *cost >= 0.0)
}

pub fn parse_catdesk_usage_jsonl(path: &Path) -> Vec<UnifiedMessage> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) => {
            warn!(
                path = %path.display(),
                error = %err,
                "Failed to open CatDesk usage ledger"
            );
            return Vec::new();
        }
    };

    BufReader::new(file)
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let line = match line {
                Ok(line) => line,
                Err(err) => {
                    warn!(
                        path = %path.display(),
                        line = index + 1,
                        error = %err,
                        "Failed to read CatDesk usage ledger row"
                    );
                    return None;
                }
            };
            if line.trim().is_empty() {
                return None;
            }

            let entry: CatDeskUsageEntry = match serde_json::from_str(&line) {
                Ok(entry) => entry,
                Err(err) => {
                    warn!(
                        path = %path.display(),
                        line = index + 1,
                        error = %err,
                        "Failed to decode CatDesk usage ledger row"
                    );
                    return None;
                }
            };
            if entry.input_tokens == 0 && entry.output_tokens == 0 {
                return None;
            }

            let timestamp = entry.timestamp_ms.min(i64::MAX as u64) as i64;
            let session_id = format!("catdesk:{}:{}", entry.timestamp_ms, index + 1);
            let mut msg = UnifiedMessage::new_with_agent(
                "hermes",
                CATDESK_MODEL_ID,
                CATDESK_PROVIDER_ID,
                session_id.clone(),
                timestamp,
                TokenBreakdown {
                    input: entry.input_tokens.min(i64::MAX as u64) as i64,
                    output: entry.output_tokens.min(i64::MAX as u64) as i64,
                    cache_read: 0,
                    cache_write: 0,
                    reasoning: 0,
                },
                0.0,
                Some(HERMES_AGENT_NAME.to_string()),
            );
            msg.dedup_key = Some(session_id);
            Some(msg)
        })
        .collect()
}

pub fn parse_hermes_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    let conn = match Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(c) => c,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to open Hermes state database"
            );
            return Vec::new();
        }
    };

    let query = r#"
        SELECT
            id,
            model,
            billing_provider,
            started_at,
            message_count,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
            reasoning_tokens,
            estimated_cost_usd,
            actual_cost_usd
        FROM sessions
        WHERE model IS NOT NULL
          AND TRIM(model) != ''
          AND (
            COALESCE(input_tokens, 0) > 0 OR
            COALESCE(output_tokens, 0) > 0 OR
            COALESCE(cache_read_tokens, 0) > 0 OR
            COALESCE(cache_write_tokens, 0) > 0 OR
            COALESCE(reasoning_tokens, 0) > 0 OR
            COALESCE(
              CASE WHEN actual_cost_usd >= 0 THEN actual_cost_usd END,
              CASE WHEN estimated_cost_usd >= 0 THEN estimated_cost_usd END,
              0
            ) > 0
          )
    "#;

    let mut stmt = match conn.prepare(query) {
        Ok(s) => s,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to prepare Hermes session query"
            );
            return Vec::new();
        }
    };

    let rows = match stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, f64>(3)?,
            row.get::<_, Option<i32>>(4)?.unwrap_or(0),
            row.get::<_, Option<i64>>(5)?.unwrap_or(0),
            row.get::<_, Option<i64>>(6)?.unwrap_or(0),
            row.get::<_, Option<i64>>(7)?.unwrap_or(0),
            row.get::<_, Option<i64>>(8)?.unwrap_or(0),
            row.get::<_, Option<i64>>(9)?.unwrap_or(0),
            row.get::<_, Option<f64>>(10)?,
            row.get::<_, Option<f64>>(11)?,
        ))
    }) {
        Ok(r) => r,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to execute Hermes session query"
            );
            return Vec::new();
        }
    };

    rows.filter_map(|row| match row {
        Ok(row) => Some(row),
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to decode Hermes session row"
            );
            None
        }
    })
    .map(
        |(
            session_id,
            model_id,
            billing_provider,
            started_at,
            message_count,
            input,
            output,
            cache_read,
            cache_write,
            reasoning,
            estimated_cost,
            actual_cost,
        )| {
            let provider = resolved_provider(billing_provider, &model_id);
            let (cost, is_provider_reported, is_estimated) =
                if let Some(cost) = valid_cost(actual_cost) {
                    (cost, true, false)
                } else if let Some(cost) = valid_cost(estimated_cost) {
                    (cost, false, true)
                } else {
                    (0.0, false, false)
                };
            let mut msg = UnifiedMessage::new_with_agent(
                "hermes",
                model_id,
                provider,
                session_id.clone(),
                timestamp_secs_to_ms(started_at),
                TokenBreakdown {
                    input: input.max(0),
                    output: output.max(0),
                    cache_read: cache_read.max(0),
                    cache_write: cache_write.max(0),
                    reasoning: reasoning.max(0),
                },
                cost,
                Some(HERMES_AGENT_NAME.to_string()),
            );
            if is_provider_reported {
                msg.mark_provider_reported_cost();
            } else if is_estimated {
                msg.mark_estimated_cost();
            }
            msg.message_count = message_count.max(0);
            msg.dedup_key = Some(session_id);
            msg
        },
    )
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use tempfile::TempDir;

    fn create_test_db() -> (TempDir, std::path::PathBuf) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("state.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                model TEXT,
                billing_provider TEXT,
                started_at REAL,
                message_count INTEGER,
                input_tokens INTEGER,
                output_tokens INTEGER,
                cache_read_tokens INTEGER,
                cache_write_tokens INTEGER,
                reasoning_tokens INTEGER,
                estimated_cost_usd REAL,
                actual_cost_usd REAL
            );",
        )
        .unwrap();
        (dir, db_path)
    }

    #[test]
    fn test_parse_hermes_cost_provenance_prefers_actual_zero() {
        let (_dir, db_path) = create_test_db();
        let conn = Connection::open(&db_path).unwrap();
        for (id, input_tokens, estimated, actual) in [
            ("actual-zero", 10_i64, Some(0.75), Some(0.0)),
            ("estimated", 10_i64, Some(0.25), None),
            ("invalid-actual", 0_i64, Some(0.4), Some(-0.1)),
            ("unknown", 10_i64, Some(-0.1), Some(-0.2)),
        ] {
            conn.execute(
                "INSERT INTO sessions (
                    id, model, billing_provider, started_at, message_count,
                    input_tokens, output_tokens, cache_read_tokens,
                    cache_write_tokens, reasoning_tokens, estimated_cost_usd,
                    actual_cost_usd
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    id,
                    "gpt-5",
                    "openai",
                    1_700_000_000.0_f64,
                    1_i32,
                    input_tokens,
                    0_i64,
                    0_i64,
                    0_i64,
                    0_i64,
                    estimated,
                    actual,
                ],
            )
            .unwrap();
        }
        drop(conn);

        let messages = parse_hermes_sqlite(&db_path);
        assert_eq!(messages.len(), 4);

        let message = |id: &str| {
            messages
                .iter()
                .find(|message| message.session_id == id)
                .unwrap()
        };
        assert_eq!(message("actual-zero").cost, 0.0);
        assert_eq!(
            message("actual-zero").cost_source,
            crate::sessions::CostSource::ProviderReported
        );
        assert_eq!(
            message("estimated").cost_source,
            crate::sessions::CostSource::Estimated
        );
        assert_eq!(message("invalid-actual").cost, 0.4);
        assert_eq!(
            message("invalid-actual").cost_source,
            crate::sessions::CostSource::Estimated
        );
        assert_eq!(
            message("unknown").cost_source,
            crate::sessions::CostSource::Unknown
        );
    }

    #[test]
    fn test_parse_catdesk_usage_jsonl_as_hermes_client() {
        let dir = TempDir::new().unwrap();
        let ledger_path = dir.path().join("usage.jsonl");
        std::fs::write(
            &ledger_path,
            concat!(
                "{\"timestampMs\":1788210000123,\"inputTokens\":12,\"outputTokens\":8,\"bucket\":\"through-gpt-5.6\"}\n",
                "{\"timestampMs\":1788210001123,\"inputTokens\":5,\"outputTokens\":7,\"bucket\":\"through-gpt-5.6\"}\n"
            ),
        )
        .unwrap();

        let messages = parse_catdesk_usage_jsonl(&ledger_path);
        assert_eq!(messages.len(), 2);
        assert!(messages.iter().all(|message| message.client == "hermes"));
        assert!(messages
            .iter()
            .all(|message| message.model_id == CATDESK_MODEL_ID));
        assert!(messages
            .iter()
            .all(|message| message.provider_id == CATDESK_PROVIDER_ID));
        assert!(messages.iter().all(|message| message.cost == 0.0));
        assert_eq!(messages[0].tokens.input, 12);
        assert_eq!(messages[0].tokens.output, 8);
        assert_eq!(messages[1].tokens.input, 5);
        assert_eq!(messages[1].tokens.output, 7);
        assert_eq!(messages[0].timestamp, 1_788_210_000_123);
        assert_eq!(messages[0].agent.as_deref(), Some(HERMES_AGENT_NAME));
        assert!(messages[0]
            .dedup_key
            .as_deref()
            .is_some_and(|key| key.starts_with("catdesk:")));
    }
}
