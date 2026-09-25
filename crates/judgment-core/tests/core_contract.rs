use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use duckjeu_judgment_core::cache::{CachePolicy, JudgmentCache};
use duckjeu_judgment_core::judgment::{
    bool_from_probability, validate_label, validate_probability, JudgmentIdentity,
    JudgmentIdentityContext, JudgmentRequest, JudgmentResult, QuestionKind,
};
use duckjeu_judgment_core::provider::{
    validate_batch_result, CapabilityStatus, ProviderAnswer, ProviderBatchResult,
    ProviderCapabilities, ProviderMetadata,
};
use duckjeu_judgment_core::serialize::{encode_struct, encode_text, TypedValue};

fn identity(state: &str) -> (JudgmentIdentity, JudgmentRequest) {
    let request = JudgmentRequest::noul(encode_text(state), "criterion").unwrap();
    let identity = JudgmentIdentity::new(
        &request,
        JudgmentIdentityContext {
            provider_namespace: "provider".into(),
            endpoint: "http://localhost".into(),
            requested_model: "model-1".into(),
            effective_model: Some("model-1".into()),
            credential_scope: 7,
            model_binding_scope: 0,
        },
    );
    (identity, request)
}

#[test]
fn canonical_encoding_orders_fields_and_distinguishes_null_from_missing() {
    let left = encode_struct(BTreeMap::from([
        ("b".into(), TypedValue::Null),
        ("a".into(), TypedValue::Int(3)),
    ]))
    .unwrap();
    let right = encode_struct(BTreeMap::from([
        ("a".into(), TypedValue::Int(3)),
        ("b".into(), TypedValue::Null),
    ]))
    .unwrap();
    let missing = encode_struct(BTreeMap::from([("a".into(), TypedValue::Int(3))])).unwrap();
    assert_eq!(left.canonical_bytes(), right.canonical_bytes());
    assert_ne!(left.canonical_bytes(), missing.canonical_bytes());
    assert_eq!(encode_text("raw").canonical_bytes(), b"raw");
}

#[test]
fn judgment_domains_and_exact_choice_membership_are_enforced() {
    assert_eq!(validate_probability(0.0).unwrap(), 0.0);
    assert!(validate_probability(1.01).is_err());
    assert!(validate_probability(f64::NAN).is_err());
    assert!(bool_from_probability(0.5));
    let choices = vec!["bird".to_string(), "shark".to_string()];
    assert_eq!(validate_label("bird", &choices).unwrap(), "bird");
    assert!(validate_label("Bird", &choices).is_err());
    assert_eq!(
        QuestionKind::Noul,
        JudgmentRequest::noul(encode_text("s"), "q").unwrap().kind
    );
}

#[test]
fn complete_identity_keeps_state_and_context_distinct() {
    let (first, request) = identity("a");
    let (different_state, _) = identity("b");
    let mut different_model = first.clone();
    different_model.requested_model = "model-2".into();
    assert_ne!(first, different_state);
    assert_ne!(first, different_model);
    assert_eq!(first.canonical_bytes, request.state.canonical_bytes());
}

#[test]
fn batch_answers_are_associated_by_key_and_invalid_coverage_fails() {
    let (first_id, first) = identity("a");
    let (second_id, mut second) = identity("b");
    let _ = (first_id, second_id);
    second.request_key = "r1".into();
    let requests = vec![first, second];
    let response = ProviderBatchResult {
        answers: vec![
            ProviderAnswer {
                request_key: "r1".into(),
                result: JudgmentResult::Noul(0.8),
            },
            ProviderAnswer {
                request_key: "r0".into(),
                result: JudgmentResult::Noul(0.2),
            },
        ],
        metadata: ProviderMetadata::default(),
    };
    assert!(validate_batch_result(&requests, &response).is_ok());
    let mut missing = response;
    missing.answers.pop();
    assert!(validate_batch_result(&requests, &missing).is_err());
}

#[test]
fn unknown_provider_capability_is_not_verified() {
    let capabilities = ProviderCapabilities {
        protocol_version: "test/v1".into(),
        noul: CapabilityStatus::Verified,
        choice: CapabilityStatus::Verified,
        multi_state_batch: CapabilityStatus::Unknown,
        max_batch_judgments: None,
        max_choices: None,
    };
    assert!(!capabilities.supports_multi_state_batch());
}

#[test]
fn cache_accepts_only_successes_and_enforces_expiry_and_capacity() {
    let policy = CachePolicy {
        max_entries: 1,
        max_bytes: 16_384,
        ttl: Duration::from_millis(10),
    };
    let now = Instant::now();
    let (first, _) = identity("a");
    let (second, _) = identity("b");
    let mut cache = JudgmentCache::default();
    assert!(!cache.insert_outcome(
        first.clone(),
        &Err(duckjeu_judgment_core::judgment::JudgmentError::provider(
            "failure"
        )),
        now,
        &policy,
    ));
    assert!(cache.insert_success(first.clone(), JudgmentResult::Noul(0.5), now, &policy));
    assert!(cache.lookup(&first, now).is_some());
    assert!(cache.insert_success(second.clone(), JudgmentResult::Noul(0.6), now, &policy));
    assert!(cache.lookup(&first, now).is_none());
    assert_eq!(cache.counters().evictions, 1);
    assert!(cache
        .lookup(&second, now + Duration::from_millis(11))
        .is_none());
    assert_eq!(cache.counters().expirations, 1);
}
