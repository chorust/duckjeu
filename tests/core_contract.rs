//! 判断语义层契约测试：不启动 DuckDB、不访问网络。

use std::collections::BTreeMap;

use duckjeu::judgment::{
    bool_from_probability, validate_label, validate_probability, JudgmentError, JudgmentRequest,
    JudgmentResult, QuestionKind,
};
use duckjeu::provider::{
    mock::MockProvider, provider_for, Provider, ProviderContext, ProviderKind,
};
use duckjeu::serialize::{
    encode_struct, encode_text, format_decimal, to_json_value, StateKind, TypedValue,
};

fn fields(pairs: &[(&str, TypedValue)]) -> BTreeMap<String, TypedValue> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_string(), value.clone()))
        .collect()
}

#[test]
fn canonical_struct_sorts_fields_by_name() {
    let a = encode_struct(fields(&[
        ("price", TypedValue::Float(1.5)),
        ("load", TypedValue::Int(3)),
    ]))
    .unwrap();
    let b = encode_struct(fields(&[
        ("load", TypedValue::Int(3)),
        ("price", TypedValue::Float(1.5)),
    ]))
    .unwrap();
    assert_eq!(a.canonical_bytes(), b.canonical_bytes());
    let text = String::from_utf8(a.canonical_bytes().to_vec()).unwrap();
    let load_at = text.find("\"load\"").unwrap();
    let price_at = text.find("\"price\"").unwrap();
    assert!(load_at < price_at, "fields must be sorted by name");
    assert!(text.starts_with("{\"canonical_version\":1,"));
}

#[test]
fn canonical_distinguishes_null_from_missing() {
    let with_null = encode_struct(fields(&[
        ("a", TypedValue::Int(1)),
        ("b", TypedValue::Null),
    ]))
    .unwrap();
    let without_null = encode_struct(fields(&[("a", TypedValue::Int(1))])).unwrap();
    assert_ne!(with_null.canonical_bytes(), without_null.canonical_bytes());
    assert!(String::from_utf8_lossy(with_null.canonical_bytes()).contains("\"b\":{\"t\":\"null\"}"));
    assert!(!String::from_utf8_lossy(without_null.canonical_bytes()).contains("\"b\""));
}

#[test]
fn canonical_keeps_type_tags_and_text_state_raw() {
    let int_state = encode_struct(fields(&[("a", TypedValue::Int(1))])).unwrap();
    let text_state = encode_struct(fields(&[("a", TypedValue::Text("1".into()))])).unwrap();
    assert_ne!(int_state.canonical_bytes(), text_state.canonical_bytes());

    let raw = encode_text("hello world");
    assert_eq!(raw.kind(), StateKind::Text);
    assert_eq!(raw.canonical_bytes(), b"hello world");
    assert_eq!(raw.as_text(), Some("hello world"));
    assert_eq!(
        to_json_value(raw.value()),
        serde_json::json!({ "t": "text", "v": "hello world" })
    );

    // 空字符串是合法文本，与 NULL 不同。
    let empty = encode_text("");
    assert_eq!(empty.canonical_bytes(), b"");
    assert_ne!(
        empty.canonical_bytes(),
        encode_struct(fields(&[("a", TypedValue::Null)]))
            .unwrap()
            .canonical_bytes()
    );
}

#[test]
fn canonical_rejects_non_finite_floats() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let err = encode_struct(fields(&[("a", TypedValue::Float(bad))])).unwrap_err();
        assert!(err.message().contains("non-finite"));
    }
}

#[test]
fn canonical_int64_precision_is_preserved() {
    let state = encode_struct(fields(&[("big", TypedValue::Int(i64::MAX))])).unwrap();
    let text = String::from_utf8(state.canonical_bytes().to_vec()).unwrap();
    assert!(text.contains(&i64::MAX.to_string()));
}

#[test]
fn noul_request_rejects_blank_criterion() {
    let state = encode_text("s");
    for bad in ["", "   ", "\t\n"] {
        assert!(JudgmentRequest::noul(state.clone(), bad).is_err());
    }
    let request = JudgmentRequest::noul(state, "the customer requests a refund").unwrap();
    assert_eq!(request.kind, QuestionKind::Noul);
    assert_eq!(request.request_key, "r0");
}

#[test]
fn choice_request_validates_candidates() {
    let state = encode_text("s");
    let ok = JudgmentRequest::choice(
        state.clone(),
        "which regime?",
        &["normal".to_string(), "scarcity".to_string()],
    )
    .unwrap();
    assert_eq!(ok.kind, QuestionKind::Choice);

    let one = JudgmentRequest::choice(state.clone(), "q", &["only".to_string()]);
    assert!(one.is_err());

    let dup = JudgmentRequest::choice(state.clone(), "q", &["a".to_string(), "a".to_string()]);
    assert!(dup.is_err());

    let blank_label =
        JudgmentRequest::choice(state.clone(), "q", &[" ".to_string(), "b".to_string()]);
    assert!(blank_label.is_err());

    let blank_question = JudgmentRequest::choice(state, "  ", &["a".to_string(), "b".to_string()]);
    assert!(blank_question.is_err());
}

#[test]
fn probability_validation_bounds() {
    assert_eq!(validate_probability(0.0).unwrap(), 0.0);
    assert_eq!(validate_probability(1.0).unwrap(), 1.0);
    assert_eq!(validate_probability(0.5).unwrap(), 0.5);
    assert!(validate_probability(1.000_001).is_err());
    assert!(validate_probability(-0.000_001).is_err());
    assert!(validate_probability(f64::NAN).is_err());
    assert!(validate_probability(f64::INFINITY).is_err());
}

#[test]
fn bool_threshold_is_inclusive_at_half() {
    assert!(bool_from_probability(0.5));
    assert!(bool_from_probability(0.500_001));
    assert!(!bool_from_probability(0.499_999));
    assert!(!bool_from_probability(0.0));
    assert!(bool_from_probability(1.0));
}

#[test]
fn label_must_be_exact_member() {
    let choices = vec!["normal".to_string(), "Scarcity".to_string()];
    assert!(validate_label("normal", &choices).is_ok());
    assert!(validate_label("Scarcity", &choices).is_ok());
    // 大小写、空白与顺序都不构成成员。
    assert!(validate_label("scarcity", &choices).is_err());
    assert!(validate_label(" normal", &choices).is_err());
    assert!(validate_label("", &choices).is_err());
}

#[test]
fn mock_provider_is_deterministic_and_in_contract() {
    let provider = MockProvider;
    let state = encode_struct(fields(&[
        ("price", TypedValue::Float(12.5)),
        ("load", TypedValue::Int(7)),
    ]))
    .unwrap();
    let request = JudgmentRequest::noul(state.clone(), "is demand high?").unwrap();
    let first = provider.judge(&request).unwrap();
    let second = provider.judge(&request).unwrap();
    assert_eq!(first, second);
    match first {
        JudgmentResult::Noul(p) => assert!((0.0..=1.0).contains(&p)),
        other => panic!("unexpected result: {other:?}"),
    }

    let choice_request = JudgmentRequest::choice(
        state,
        "which regime?",
        &["normal".into(), "scarcity".into(), "oversupply".into()],
    )
    .unwrap();
    let chosen = provider.judge(&choice_request).unwrap();
    match chosen {
        JudgmentResult::Choice(label) => assert!(choice_request.choices.contains(&label)),
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn mock_provider_separates_states_and_questions() {
    let provider = MockProvider;
    let a = JudgmentRequest::noul(encode_text("alpha"), "q").unwrap();
    let b = JudgmentRequest::noul(encode_text("beta"), "q").unwrap();
    let c = JudgmentRequest::noul(encode_text("alpha"), "other").unwrap();
    let pa = provider.judge(&a).unwrap();
    let pb = provider.judge(&b).unwrap();
    let pc = provider.judge(&c).unwrap();
    assert_ne!(pa, pb);
    assert_ne!(pa, pc);
}

#[test]
fn typesafe_requires_credential_and_is_explicit() {
    let mut ctx = ProviderContext {
        provider: ProviderKind::Typesafe,
        ..ProviderContext::default()
    };
    ctx.credential = None;
    let err = match provider_for(&ctx) {
        Ok(_) => panic!("typesafe provider must require a credential"),
        Err(err) => err,
    };
    assert_eq!(err.kind(), duckjeu::judgment::ErrorKind::Configuration);

    ctx.credential = Some("token".to_string());
    assert!(provider_for(&ctx).is_ok());

    // 默认上下文是离线 mock。
    let default_provider = provider_for(&ProviderContext::default()).unwrap();
    let request = JudgmentRequest::noul(encode_text("x"), "q").unwrap();
    assert!(default_provider.judge(&request).is_ok());
}

#[test]
fn provider_kind_parsing_is_strict() {
    assert_eq!(ProviderKind::parse("mock").unwrap(), ProviderKind::Mock);
    assert_eq!(
        ProviderKind::parse(" TypeSafe ").unwrap(),
        ProviderKind::Typesafe
    );
    let err: JudgmentError = ProviderKind::parse("openai").unwrap_err();
    assert!(err.message().contains("unknown duckjeu_provider"));
}

#[test]
fn decimal_rendering_is_exact() {
    assert_eq!(format_decimal(15, 1), "1.5");
    assert_eq!(format_decimal(-15, 1), "-1.5");
    assert_eq!(format_decimal(5, 3), "0.005");
    assert_eq!(format_decimal(-5, 3), "-0.005");
    assert_eq!(format_decimal(123456, 0), "123456");
    assert_eq!(format_decimal(0, 2), "0.00");
    assert_eq!(format_decimal(i128::MAX, 0), i128::MAX.to_string());
}

#[test]
fn decimal_and_big_integers_have_own_type_tags() {
    let decimal = encode_struct(fields(&[("price", TypedValue::Decimal("1.5".into()))])).unwrap();
    let float = encode_struct(fields(&[("price", TypedValue::Float(1.5))])).unwrap();
    assert_ne!(decimal.canonical_bytes(), float.canonical_bytes());
    assert!(String::from_utf8_lossy(decimal.canonical_bytes()).contains("\"t\":\"decimal\""));

    let big = encode_struct(fields(&[(
        "big",
        TypedValue::BigInt("170141183460469231731687303715884105727".into()),
    )]))
    .unwrap();
    assert!(String::from_utf8_lossy(big.canonical_bytes())
        .contains("170141183460469231731687303715884105727"));
}
