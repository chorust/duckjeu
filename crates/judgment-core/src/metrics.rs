//! Versioned, redacted execution profile aggregation and rendering.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueryCounts {
    pub input_rows: u64,
    pub null_rows: u64,
    pub valid_rows: u64,
    pub unique_judgments: u64,
    pub deduplicated_rows: u64,
    pub cache_hit_judgments: u64,
    pub provider_invocations: u64,
    pub external_requests: u64,
    pub batched_requests: u64,
    pub batch_size_histogram: BTreeMap<usize, u64>,
    pub failed_requests: u64,
    pub reported_models: BTreeSet<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub usage_response_count: u64,
    pub usage_missing_count: u64,
    pub serialization_time_ns: u128,
    pub serialization_observed_count: u64,
    pub external_roundtrip_ns: u128,
    pub external_roundtrip_count: u64,
    pub inference_time_ms: u128,
    pub inference_observed_count: u64,
    pub inference_missing_count: u64,
}

pub struct ProfileReport<'a> {
    pub status: &'a str,
    pub provider: Option<&'a str>,
    pub requested_model: Option<&'a str>,
    pub counts: &'a QueryCounts,
    pub elapsed: Duration,
    pub detailed_timing: bool,
}

fn millis(nanoseconds: u128) -> f64 {
    nanoseconds as f64 / 1_000_000.0
}

/// Produces a JSON report with no user state, questions, endpoints, credentials, or error text.
pub fn render_profile(report: ProfileReport<'_>) -> Result<String, serde_json::Error> {
    let counts = report.counts;
    let usage_complete = counts.external_requests > 0
        && counts.usage_missing_count == 0
        && counts.usage_response_count == counts.external_requests;
    let usage_input = usage_complete.then_some(counts.input_tokens).flatten();
    let usage_output = usage_complete.then_some(counts.output_tokens).flatten();
    let inference_complete = counts.external_requests > 0
        && counts.inference_missing_count == 0
        && counts.inference_observed_count == counts.external_requests;
    let timing = |value: f64, observed: bool| {
        if report.detailed_timing && observed {
            serde_json::Value::from(value)
        } else {
            serde_json::Value::Null
        }
    };

    let typesafe_priced = report.provider == Some("typesafe");
    let cost_unknown_reason = if !typesafe_priced {
        "provider_pricing_not_applicable_or_unverified"
    } else if counts.external_requests == 0 {
        "no_external_requests"
    } else if counts.failed_requests > 0 {
        "failed_requests_may_be_billable"
    } else if !usage_complete || usage_input.is_none() {
        "provider_usage_unavailable_or_incomplete"
    } else {
        ""
    };
    let cost = if cost_unknown_reason.is_empty() {
        let amount = usage_input.unwrap_or_default() as f64 * 0.042 / 1_000_000.0;
        serde_json::json!({
            "amount": amount,
            "currency": "USD",
            "billing_unit": "USD per 1,000,000 input tokens",
            "unit_rate": 0.042,
            "rate_source": "https://typesafe.ai/blog/introducing-system-one-models-and-jev",
            "rate_observed_at": "2026-09-23",
            "unknown_reason": null
        })
    } else {
        serde_json::json!({
            "amount": null,
            "currency": if typesafe_priced { serde_json::Value::from("USD") } else { serde_json::Value::Null },
            "billing_unit": if typesafe_priced { serde_json::Value::from("USD per 1,000,000 input tokens") } else { serde_json::Value::Null },
            "unit_rate": if typesafe_priced { serde_json::Value::from(0.042) } else { serde_json::Value::Null },
            "rate_source": if typesafe_priced { serde_json::Value::from("https://typesafe.ai/blog/introducing-system-one-models-and-jev") } else { serde_json::Value::Null },
            "rate_observed_at": if typesafe_priced { serde_json::Value::from("2026-09-23") } else { serde_json::Value::Null },
            "unknown_reason": cost_unknown_reason
        })
    };

    let histogram: BTreeMap<String, u64> = counts
        .batch_size_histogram
        .iter()
        .map(|(size, count)| (size.to_string(), *count))
        .collect();
    let report = serde_json::json!({
        "schema_version": 1,
        "status": report.status,
        "provider": report.provider,
        "requested_model": report.requested_model,
        "reported_models": counts.reported_models,
        "counts": {
            "input_rows": counts.input_rows,
            "null_rows": counts.null_rows,
            "valid_rows": counts.valid_rows,
            "unique_judgments": counts.unique_judgments,
            "deduplicated_rows": counts.deduplicated_rows,
            "cache_hit_judgments": counts.cache_hit_judgments,
            "provider_invocations": counts.provider_invocations,
            "external_requests": counts.external_requests,
            "batched_requests": counts.batched_requests,
            "batch_size_histogram": histogram,
            "failed_requests": counts.failed_requests
        },
        "timing_ms": {
            "query_wall": millis(report.elapsed.as_nanos()),
            "serialization_sum": timing(millis(counts.serialization_time_ns), counts.serialization_observed_count == counts.external_requests && counts.external_requests > 0),
            "external_roundtrip_sum": timing(millis(counts.external_roundtrip_ns), counts.external_roundtrip_count == counts.external_requests && counts.external_requests > 0),
            "server_inference_sum": if inference_complete { serde_json::Value::from(counts.inference_time_ms as f64) } else { serde_json::Value::Null },
            "pure_network_sum": null
        },
        "usage": {
            "input_tokens": usage_input,
            "output_tokens": usage_output,
            "missing_response_count": counts.usage_missing_count
        },
        "cost_estimate": cost
    });
    serde_json::to_string(&report)
}
