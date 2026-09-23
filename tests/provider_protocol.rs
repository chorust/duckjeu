//! TypeSafe adapter 的请求构造与响应校验契约测试（不访问网络）。

use duckjeu::judgment::{JudgmentRequest, JudgmentResult};
use duckjeu::provider::typesafe::TypesafeProvider;
use duckjeu::serialize::{encode_struct, encode_text, TypedValue};

use std::collections::BTreeMap;

fn provider() -> TypesafeProvider {
    TypesafeProvider::new(
        "http://127.0.0.1:1/v1/systemone".to_string(),
        "jev-latest".to_string(),
        1_000,
        1_048_576,
        "test-token".to_string(),
    )
}

fn struct_state() -> duckjeu::serialize::CanonicalState {
    let mut fields = BTreeMap::new();
    fields.insert("price".to_string(), TypedValue::Float(1.5));
    fields.insert("load".to_string(), TypedValue::Null);
    encode_struct(fields).unwrap()
}

#[test]
fn noul_body_carries_condition_and_question_key() {
    let request = JudgmentRequest::noul(struct_state(), "is demand high?").unwrap();
    let body = provider().build_body(&request);
    assert_eq!(body["model"], "jev-latest");
    assert_eq!(body["state"]["condition"], "is demand high?");
    assert!(body["state"]["rows"].is_array());
    assert_eq!(body["state"]["rows"][0]["t"], "struct");
    assert_eq!(body["questions"]["r0"]["type"], "noul");
}

#[test]
fn text_state_is_sent_as_raw_string() {
    let request =
        JudgmentRequest::noul(encode_text("the ticket asks for a refund"), "refund?").unwrap();
    let body = provider().build_body(&request);
    assert_eq!(
        body["state"]["rows"][0],
        serde_json::json!("the ticket asks for a refund")
    );
}

#[test]
fn choice_body_carries_candidate_map() {
    let request = JudgmentRequest::choice(
        encode_text("state"),
        "which regime?",
        &["normal".to_string(), "scarcity".to_string()],
    )
    .unwrap();
    let body = provider().build_body(&request);
    assert_eq!(body["questions"]["r0"]["type"], "choice");
    assert_eq!(
        body["questions"]["r0"]["instructions"],
        "Caller question: which regime?\nSelect the single candidate label that best answers this question for state.rows[0]."
    );
    let criteria = body["questions"]["r0"]["criteria"].as_object().unwrap();
    assert!(body["state"].get("criteria").is_none());
    assert_eq!(criteria.len(), 2);
    assert!(criteria["normal"].is_null());
    assert!(criteria["scarcity"].is_null());
}

#[test]
fn noul_response_validation() {
    let provider = provider();
    let request = JudgmentRequest::noul(encode_text("s"), "q").unwrap();

    let ok = provider
        .parse_response(
            &request,
            r#"{"model":"jev-1.13.0","answers":{"r0":{"type":"noul","noul":0.83}}}"#,
        )
        .unwrap();
    assert_eq!(ok, JudgmentResult::Noul(0.83));

    // 边界 0 与 1 合法。
    assert!(provider
        .parse_response(&request, r#"{"answers":{"r0":{"type":"noul","noul":0}}}"#)
        .is_ok());
    assert!(provider
        .parse_response(&request, r#"{"answers":{"r0":{"type":"noul","noul":1}}}"#)
        .is_ok());

    for bad in [
        r#"{"answers":{"r0":{"type":"noul","noul":1.5}}}"#, // 越界
        r#"{"answers":{"r0":{"type":"noul","noul":-0.1}}}"#, // 越界
        r#"{"answers":{"r0":{"type":"noul","noul":"0.5"}}}"#, // 类型错
        r#"{"answers":{"r0":{"type":"choice","choice":"normal"}}}"#, // 类型错
        r#"{"answers":{}}"#,                                // 缺失
        r#"{"answers":{"r0":0.5,"r1":0.5}}"#,               // 多余
        r#"{"answers":{"r9":0.5}}"#,                        // 未知身份
        r#"{"model":"x"}"#,                                 // 无 answers
        r#"not json"#,                                      // 非 JSON
    ] {
        assert!(
            provider.parse_response(&request, bad).is_err(),
            "must reject: {bad}"
        );
    }
}

#[test]
fn choice_response_must_be_candidate_member() {
    let provider = provider();
    let request = JudgmentRequest::choice(
        encode_text("s"),
        "q",
        &["normal".to_string(), "scarcity".to_string()],
    )
    .unwrap();

    let ok = provider
        .parse_response(
            &request,
            r#"{"answers":{"r0":{"type":"choice","choice":"scarcity","confidence":0.8,"probabilities":{"normal":0.2,"scarcity":0.8}}}}"#,
        )
        .unwrap();
    assert_eq!(ok, JudgmentResult::Choice("scarcity".to_string()));

    for bad in [
        r#"{"answers":{"r0":{"type":"choice","choice":"oversupply"}}}"#, // 未知 label
        r#"{"answers":{"r0":{"type":"noul","noul":0.7}}}"#,              // 类型错
        r#"{"answers":{}}"#,                                             // 缺失
    ] {
        assert!(
            provider.parse_response(&request, bad).is_err(),
            "must reject: {bad}"
        );
    }
}
