//! 完整判断身份契约：哈希相同只能缩小查找范围，不能代替完整相等比较。

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use duckjeu::judgment::{
    validate_judgment_result, JudgmentIdentity, JudgmentIdentityContext, JudgmentRequest,
    JudgmentResult,
};
use duckjeu::serialize::{encode_struct, encode_text, TypedValue};

fn context() -> JudgmentIdentityContext {
    JudgmentIdentityContext {
        provider_namespace: "typesafe".into(),
        endpoint: "https://example.invalid/v1/systemone".into(),
        requested_model: "jev-latest".into(),
        effective_model: None,
        credential_scope: 7,
        model_binding_scope: 11,
    }
}

fn identity(request: &JudgmentRequest) -> JudgmentIdentity {
    JudgmentIdentity::new(request, context())
}

fn noul(state: duckjeu::serialize::CanonicalState, criterion: &str) -> JudgmentRequest {
    JudgmentRequest::noul(state, criterion).unwrap()
}

fn choice(state: duckjeu::serialize::CanonicalState, labels: &[&str]) -> JudgmentRequest {
    JudgmentRequest::choice(
        state,
        "select a label",
        &labels
            .iter()
            .map(|label| (*label).to_string())
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

#[test]
fn complete_identity_matches_only_the_same_request_and_context() {
    let base = identity(&noul(encode_text("same state"), "is valid?"));
    assert_eq!(
        base,
        identity(&noul(encode_text("same state"), "is valid?"))
    );

    let mut changed = base.clone();
    changed.canonical_version += 1;
    assert_ne!(
        base, changed,
        "canonical encoding version is part of identity"
    );

    assert_ne!(
        base,
        identity(&noul(encode_text("other state"), "is valid?"))
    );
    assert_ne!(
        base,
        identity(&noul(encode_text("same state"), "another question"))
    );
    assert_ne!(
        base,
        identity(&choice(encode_text("same state"), &["yes", "no"])),
        "question kind and candidate semantics are part of identity"
    );

    let mut different_provider = context();
    different_provider.provider_namespace = "another-provider".into();
    assert_ne!(
        base,
        JudgmentIdentity::new(
            &noul(encode_text("same state"), "is valid?"),
            different_provider
        )
    );

    let mut different_endpoint = context();
    different_endpoint.endpoint.push_str("/other");
    assert_ne!(
        base,
        JudgmentIdentity::new(
            &noul(encode_text("same state"), "is valid?"),
            different_endpoint
        )
    );

    let mut different_requested_model = context();
    different_requested_model.requested_model = "jev-v2".into();
    assert_ne!(
        base,
        JudgmentIdentity::new(
            &noul(encode_text("same state"), "is valid?"),
            different_requested_model
        )
    );

    let mut different_effective_model = context();
    different_effective_model.effective_model = Some("jev-2026-09".into());
    assert_ne!(
        base,
        JudgmentIdentity::new(
            &noul(encode_text("same state"), "is valid?"),
            different_effective_model
        )
    );

    let mut different_credential_scope = context();
    different_credential_scope.credential_scope += 1;
    assert_ne!(
        base,
        JudgmentIdentity::new(
            &noul(encode_text("same state"), "is valid?"),
            different_credential_scope
        )
    );

    let mut different_unknown_model_context = context();
    different_unknown_model_context.model_binding_scope += 1;
    assert_ne!(
        base,
        JudgmentIdentity::new(
            &noul(encode_text("same state"), "is valid?"),
            different_unknown_model_context
        ),
        "unresolved model aliases cannot bridge separate execution contexts"
    );
}

#[test]
fn text_numeric_type_internal_null_and_missing_field_are_distinct() {
    let text_state = encode_text("1");
    let versioned = text_state.versioned_canonical_bytes();
    assert_eq!(&versioned[..4], b"DJST");
    assert_eq!(
        u32::from_be_bytes(versioned[4..8].try_into().unwrap()),
        text_state.canonical_version()
    );
    assert_eq!(versioned[8], 1, "text state has its own type tag");

    let text = identity(&noul(text_state, "q"));
    let numeric = identity(&noul(
        encode_struct([("value".to_string(), TypedValue::Int(1))].into()).unwrap(),
        "q",
    ));
    assert_ne!(
        text, numeric,
        "text and numeric states with similar display differ"
    );

    let typed_text = encode_struct(
        [("value".to_string(), TypedValue::Text("1".into()))]
            .into_iter()
            .collect(),
    )
    .unwrap();
    let typed_int = encode_struct(
        [("value".to_string(), TypedValue::Int(1))]
            .into_iter()
            .collect(),
    )
    .unwrap();
    assert_ne!(typed_text.canonical_bytes(), typed_int.canonical_bytes());

    let with_null = encode_struct(
        [
            ("value".to_string(), TypedValue::Int(1)),
            ("optional".to_string(), TypedValue::Null),
        ]
        .into_iter()
        .collect(),
    )
    .unwrap();
    let missing = encode_struct(
        [("value".to_string(), TypedValue::Int(1))]
            .into_iter()
            .collect(),
    )
    .unwrap();
    assert_ne!(
        identity(&noul(with_null, "q")),
        identity(&noul(missing, "q"))
    );
}

#[test]
fn struct_field_order_normalizes_but_choice_order_does_not() {
    let first = encode_struct(
        [
            ("z".to_string(), TypedValue::Int(2)),
            ("a".to_string(), TypedValue::Bool(true)),
        ]
        .into_iter()
        .collect(),
    )
    .unwrap();
    let reordered = encode_struct(
        [
            ("a".to_string(), TypedValue::Bool(true)),
            ("z".to_string(), TypedValue::Int(2)),
        ]
        .into_iter()
        .collect(),
    )
    .unwrap();
    assert_eq!(
        identity(&noul(first, "q")),
        identity(&noul(reordered, "q")),
        "STRUCT field insertion order is normalized"
    );

    assert_ne!(
        identity(&choice(encode_text("s"), &["red", "blue"])),
        identity(&choice(encode_text("s"), &["blue", "red"])),
        "candidate order remains part of choice semantics"
    );
}

#[derive(Default)]
struct ConstantHasher;

impl Hasher for ConstantHasher {
    fn finish(&self) -> u64 {
        0
    }

    fn write(&mut self, _bytes: &[u8]) {}
}

#[test]
fn forced_hash_collisions_do_not_merge_distinct_identities() {
    let mut map: HashMap<_, _, BuildHasherDefault<ConstantHasher>> = HashMap::default();
    let alpha = identity(&noul(encode_text("s"), "alpha"));
    let beta = identity(&noul(encode_text("s"), "beta"));

    map.insert(alpha.clone(), "first");
    map.insert(beta.clone(), "second");

    assert_eq!(map.len(), 2);
    assert_eq!(map.get(&alpha), Some(&"first"));
    assert_eq!(map.get(&beta), Some(&"second"));
}

#[test]
fn executor_result_validation_rejects_wrong_type_and_out_of_domain_values() {
    let noul_request = noul(encode_text("s"), "q");
    assert!(validate_judgment_result(&noul_request, &JudgmentResult::Noul(0.5)).is_ok());
    assert!(validate_judgment_result(&noul_request, &JudgmentResult::Noul(1.1)).is_err());
    assert!(
        validate_judgment_result(&noul_request, &JudgmentResult::Choice("yes".into())).is_err()
    );

    let choice_request = choice(encode_text("s"), &["yes", "no"]);
    assert!(
        validate_judgment_result(&choice_request, &JudgmentResult::Choice("yes".into())).is_ok()
    );
    assert!(
        validate_judgment_result(&choice_request, &JudgmentResult::Choice("maybe".into())).is_err()
    );
    assert!(validate_judgment_result(&choice_request, &JudgmentResult::Noul(0.5)).is_err());
}
