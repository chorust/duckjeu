//! Connection cache policy contracts: successful results only, bounded memory and freshness.

use std::time::{Duration, Instant};

use duckjeu::cache::{CachePolicy, JudgmentCache};
use duckjeu::judgment::{
    JudgmentError, JudgmentIdentity, JudgmentIdentityContext, JudgmentRequest, JudgmentResult,
};
use duckjeu::serialize::encode_text;

fn identity(state: &str, credential_scope: u64, model_binding_scope: u64) -> JudgmentIdentity {
    let request = JudgmentRequest::noul(encode_text(state), "is valid?").unwrap();
    JudgmentIdentity::new(
        &request,
        JudgmentIdentityContext {
            provider_namespace: "test-provider".into(),
            endpoint: "https://example.invalid/judge".into(),
            requested_model: "jev-1.13.0".into(),
            effective_model: None,
            credential_scope,
            model_binding_scope,
        },
    )
}

fn policy(max_entries: usize, max_bytes: usize, ttl_ms: u64) -> CachePolicy {
    CachePolicy {
        max_entries,
        max_bytes,
        ttl: Duration::from_millis(ttl_ms),
    }
}

#[test]
fn only_successful_results_are_cached() {
    let now = Instant::now();
    let mut cache = JudgmentCache::default();
    let id = identity("state", 1, 1);
    let limits = policy(8, 16_384, 1_000);

    assert!(cache.insert_outcome(id.clone(), &Ok(JudgmentResult::Noul(0.7)), now, &limits));
    assert_eq!(cache.lookup(&id, now), Some(JudgmentResult::Noul(0.7)));
    assert!(!cache.insert_outcome(
        identity("failed", 1, 1),
        &Err(JudgmentError::provider("offline")),
        now,
        &limits
    ));
    assert_eq!(cache.lookup(&identity("failed", 1, 1), now), None);
}

#[test]
fn ttl_expires_at_the_boundary_and_stale_entries_are_removed() {
    let now = Instant::now();
    let expires = now + Duration::from_millis(100);
    let mut cache = JudgmentCache::default();
    let id = identity("state", 1, 1);
    assert!(cache.insert_outcome(
        id.clone(),
        &Ok(JudgmentResult::Choice("yes".into())),
        now,
        &policy(4, 16_384, 100)
    ));

    assert_eq!(
        cache.lookup(&id, expires - Duration::from_nanos(1)),
        Some(JudgmentResult::Choice("yes".into()))
    );
    assert_eq!(cache.lookup(&id, expires), None);
    assert_eq!(cache.len(expires), 0);
    assert_eq!(cache.counters().expirations, 1);
}

#[test]
fn entry_limit_uses_lru_order_and_byte_limit_is_enforced_separately() {
    let now = Instant::now();
    let limits = policy(2, 64_000, 10_000);
    let mut cache = JudgmentCache::default();
    let a = identity("a", 1, 1);
    let b = identity("b", 1, 1);
    let c = identity("c", 1, 1);
    for id in [&a, &b] {
        assert!(cache.insert_outcome(id.clone(), &Ok(JudgmentResult::Noul(0.5)), now, &limits));
    }
    assert_eq!(cache.lookup(&a, now), Some(JudgmentResult::Noul(0.5)));
    assert!(cache.insert_outcome(c.clone(), &Ok(JudgmentResult::Noul(0.8)), now, &limits));
    assert_eq!(cache.lookup(&a, now), Some(JudgmentResult::Noul(0.5)));
    assert_eq!(
        cache.lookup(&b, now),
        None,
        "least recently used entry is evicted"
    );
    assert_eq!(cache.lookup(&c, now), Some(JudgmentResult::Noul(0.8)));
    assert_eq!(cache.counters().evictions, 1);

    let size = JudgmentCache::estimated_entry_bytes(&a, &JudgmentResult::Noul(0.5));
    let mut byte_limited = JudgmentCache::default();
    assert!(!byte_limited.insert_outcome(
        a.clone(),
        &Ok(JudgmentResult::Noul(0.5)),
        now,
        &policy(10, size.saturating_sub(1), 10_000)
    ));
    assert!(byte_limited.is_empty());
}

#[test]
fn complete_identity_keeps_credentials_provider_endpoint_and_unknown_alias_scopes_separate() {
    let now = Instant::now();
    let mut cache = JudgmentCache::default();
    let id = identity("same", 10, 1);
    assert!(cache.insert_outcome(
        id.clone(),
        &Ok(JudgmentResult::Noul(0.3)),
        now,
        &policy(8, 16_384, 1_000)
    ));

    assert_eq!(cache.lookup(&identity("same", 11, 1), now), None);
    assert_eq!(cache.lookup(&identity("same", 10, 2), now), None);
    let request = JudgmentRequest::noul(encode_text("same"), "is valid?").unwrap();
    let mut other = JudgmentIdentity::new(
        &request,
        JudgmentIdentityContext {
            provider_namespace: "other-provider".into(),
            endpoint: "https://example.invalid/judge".into(),
            requested_model: "jev-1.13.0".into(),
            effective_model: Some("jev-1.13.0".into()),
            credential_scope: 10,
            model_binding_scope: 1,
        },
    );
    assert_eq!(cache.lookup(&other, now), None);
    other.endpoint.push_str("/v2");
    assert_eq!(cache.lookup(&other, now), None);
}

#[test]
fn clear_reports_removed_entries_and_resets_memory_usage() {
    let now = Instant::now();
    let mut cache = JudgmentCache::default();
    let limits = policy(4, 16_384, 1_000);
    for state in ["a", "b", "c"] {
        assert!(cache.insert_outcome(
            identity(state, 1, 1),
            &Ok(JudgmentResult::Noul(0.5)),
            now,
            &limits
        ));
    }
    assert_eq!(cache.clear(), 3);
    assert!(cache.is_empty());
    assert_eq!(cache.estimated_bytes(), 0);
}
