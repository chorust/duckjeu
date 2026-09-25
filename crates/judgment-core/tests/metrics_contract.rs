use duckjeu_judgment_core::metrics::{render_profile, ProfileReport, QueryCounts};
use std::collections::BTreeSet;
use std::time::Duration;

fn report<'a>(counts: &'a QueryCounts, provider: &'a str) -> serde_json::Value {
    serde_json::from_str(
        &render_profile(ProfileReport {
            status: "completed",
            provider: Some(provider),
            requested_model: Some("jev-1.13.0"),
            counts,
            elapsed: Duration::from_millis(10),
            detailed_timing: true,
        })
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn unknown_inference_and_network_split_remain_null() {
    let counts = QueryCounts {
        external_requests: 1,
        usage_response_count: 1,
        input_tokens: Some(1_000),
        output_tokens: Some(0),
        usage_missing_count: 0,
        external_roundtrip_ns: 25_000_000,
        external_roundtrip_count: 1,
        ..QueryCounts::default()
    };
    let value = report(&counts, "typesafe");
    assert_eq!(value["timing_ms"]["external_roundtrip_sum"], 25.0);
    assert!(value["timing_ms"]["server_inference_sum"].is_null());
    assert!(value["timing_ms"]["pure_network_sum"].is_null());
}

#[test]
fn parallel_request_time_is_not_clamped_to_query_wall_time() {
    let counts = QueryCounts {
        external_requests: 2,
        external_roundtrip_ns: 30_000_000,
        external_roundtrip_count: 2,
        serialization_time_ns: 4_000_000,
        serialization_observed_count: 2,
        ..QueryCounts::default()
    };
    let serialized = render_profile(ProfileReport {
        status: "completed",
        provider: Some("typesafe"),
        requested_model: Some("jev-1.13.0"),
        counts: &counts,
        elapsed: Duration::from_millis(10),
        detailed_timing: true,
    })
    .unwrap();
    let value: serde_json::Value = serde_json::from_str(&serialized).unwrap();
    assert_eq!(value["timing_ms"]["query_wall"], 10.0);
    assert_eq!(value["timing_ms"]["external_roundtrip_sum"], 30.0);
    assert_eq!(value["timing_ms"]["serialization_sum"], 4.0);
}

#[test]
fn incomplete_or_failed_usage_keeps_estimated_cost_unknown() {
    let counts = QueryCounts {
        external_requests: 1,
        failed_requests: 1,
        usage_missing_count: 1,
        ..QueryCounts::default()
    };
    let value = report(&counts, "typesafe");
    assert!(value["cost_estimate"]["amount"].is_null());
    assert_eq!(
        value["cost_estimate"]["unknown_reason"],
        "failed_requests_may_be_billable"
    );
}

#[test]
fn typesafe_cost_uses_reported_input_tokens_and_documented_rate() {
    let counts = QueryCounts {
        external_requests: 1,
        usage_response_count: 1,
        usage_missing_count: 0,
        input_tokens: Some(1_000_000),
        output_tokens: Some(10),
        reported_models: BTreeSet::from(["jev-1.13.0".to_string()]),
        ..QueryCounts::default()
    };
    let value = report(&counts, "typesafe");
    assert_eq!(value["cost_estimate"]["amount"], 0.042);
    assert_eq!(value["cost_estimate"]["currency"], "USD");
    assert_eq!(value["reported_models"][0], "jev-1.13.0");
}

#[test]
fn non_typesafe_provider_does_not_inherit_remote_pricing() {
    let counts = QueryCounts {
        external_requests: 1,
        ..QueryCounts::default()
    };
    let value = report(&counts, "selfhosted");
    assert!(value["cost_estimate"]["amount"].is_null());
    assert!(value["cost_estimate"]["unit_rate"].is_null());
}

#[test]
fn profile_contains_only_approved_aggregates() {
    let counts = QueryCounts {
        input_rows: 5,
        valid_rows: 5,
        unique_judgments: 2,
        deduplicated_rows: 3,
        ..QueryCounts::default()
    };
    let serialized = render_profile(ProfileReport {
        status: "completed",
        provider: Some("typesafe"),
        requested_model: Some("jev-latest"),
        counts: &counts,
        elapsed: Duration::from_millis(1),
        detailed_timing: false,
    })
    .unwrap();
    assert!(!serialized.contains("private customer state"));
    assert!(!serialized.contains("secret-token"));
    let value: serde_json::Value = serde_json::from_str(&serialized).unwrap();
    assert_eq!(value["counts"]["input_rows"], 5);
    assert!(value["timing_ms"]["serialization_sum"].is_null());
}
