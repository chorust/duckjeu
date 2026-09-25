//! TypeSafe adapter 的请求构造与响应校验契约测试（不访问网络）。

use duckjeu::judgment::{JudgmentRequest, JudgmentResult};
use duckjeu::provider::localjev::LocalJevProvider;
use duckjeu::provider::typesafe::TypesafeProvider;
use duckjeu::provider::{CapabilityStatus, Provider};
use duckjeu::serialize::{encode_struct, encode_text, TypedValue};

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

fn provider() -> TypesafeProvider {
    provider_with_model("jev-latest")
}

fn provider_with_model(model: &str) -> TypesafeProvider {
    TypesafeProvider::new(
        "http://127.0.0.1:1/v1/systemone".to_string(),
        model.to_string(),
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

fn read_http_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let count = stream.read(&mut chunk).unwrap();
        if count == 0 {
            panic!("client closed before sending the complete request");
        }
        bytes.extend_from_slice(&chunk[..count]);
        let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&bytes[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or_default();
        if bytes.len() >= header_end + 4 + content_length {
            return String::from_utf8_lossy(&bytes).into_owned();
        }
    }
}

fn request_body(request: &str) -> serde_json::Value {
    serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap()
}

fn serve_one_request(listener: TcpListener, response_body: &str) -> Option<String> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let request = read_http_request(&mut stream);
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response_body.len(),
                    response_body
                )
                .unwrap();
                return Some(request);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return None;
                }
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("local HTTP fixture accept failed: {error}"),
        }
    }
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

#[test]
fn typesafe_preserves_requested_and_reported_model_and_usage() {
    let provider = provider();
    let request = JudgmentRequest::noul(encode_text("s"), "q").unwrap();
    let response = provider
        .parse_batch_response(
            std::slice::from_ref(&request),
            r#"{"model":"jev-1.13.0","usage":{"input_tokens":123,"output_tokens":7},"answers":{"r0":{"type":"noul","noul":0.6}}}"#,
        )
        .unwrap();

    assert_eq!(response.answers[0].request_key, "r0");
    assert_eq!(response.answers[0].result, JudgmentResult::Noul(0.6));
    assert_eq!(
        response.metadata.requested_model.as_deref(),
        Some("jev-latest")
    );
    assert_eq!(
        response.metadata.effective_model.as_deref(),
        Some("jev-1.13.0")
    );
    let usage = response.metadata.usage.unwrap();
    assert_eq!(usage.input_tokens, Some(123));
    assert_eq!(usage.output_tokens, Some(7));
    assert_eq!(response.metadata.inference_time_ms, None);
}

#[test]
fn typesafe_multi_state_batch_is_verified_only_for_live_tested_models() {
    let capabilities = provider().capabilities();
    assert_eq!(capabilities.noul, CapabilityStatus::Verified);
    assert_eq!(capabilities.choice, CapabilityStatus::Verified);
    assert_eq!(capabilities.multi_state_batch, CapabilityStatus::Verified);
    assert_eq!(
        provider_with_model("jev-1.13.0")
            .capabilities()
            .multi_state_batch,
        CapabilityStatus::Verified
    );
    assert_eq!(
        provider_with_model("unverified-model")
            .capabilities()
            .multi_state_batch,
        CapabilityStatus::Unknown
    );
    assert_eq!(
        provider().cache_model_identity("jev-latest"),
        None,
        "a mutable alias is not a cross-query cache identity"
    );
    assert_eq!(
        provider().cache_model_identity("jev-1.13.0"),
        Some("jev-1.13.0".to_string()),
        "only explicit versioned model identifiers are cacheable"
    );
    assert_eq!(provider().cache_model_identity("custom-alias"), None);
}

#[test]
fn typesafe_builds_choice_composite_rows_with_per_row_questions() {
    let provider = provider();
    let choices = vec!["bird".to_string(), "shark".to_string()];
    let mut first =
        JudgmentRequest::choice(encode_text("bird"), "which animal?", &choices).unwrap();
    first.request_key = "r0".into();
    let mut second =
        JudgmentRequest::choice(encode_text("shark"), "which animal?", &choices).unwrap();
    second.request_key = "r1".into();

    let body = provider.build_batch_body(&[first, second]).unwrap();
    assert_eq!(body["state"]["rows"], serde_json::json!(["bird", "shark"]));
    assert_eq!(body["questions"].as_object().unwrap().len(), 2);
    assert!(body["questions"]["r0"]["instructions"]
        .as_str()
        .unwrap()
        .contains("state.rows[0]"));
    assert!(body["questions"]["r1"]["instructions"]
        .as_str()
        .unwrap()
        .contains("state.rows[1]"));
    for key in ["r0", "r1"] {
        assert_eq!(body["questions"][key]["type"], "choice");
        let criteria = body["questions"][key]["criteria"].as_object().unwrap();
        assert_eq!(criteria.len(), 2);
        assert!(criteria["bird"].is_null());
        assert!(criteria["shark"].is_null());
    }
}

#[test]
fn typesafe_rejects_incompatible_or_duplicate_batch_questions_before_dispatch() {
    let provider = provider();
    let mut first = JudgmentRequest::noul(encode_text("paid"), "invoice is paid").unwrap();
    first.request_key = "r0".into();
    let mut different_criterion =
        JudgmentRequest::noul(encode_text("unpaid"), "invoice is overdue").unwrap();
    different_criterion.request_key = "r1".into();
    assert!(provider
        .build_batch_body(&[first.clone(), different_criterion])
        .is_err());

    let duplicate_key = JudgmentRequest {
        request_key: first.request_key.clone(),
        state: encode_text("unpaid"),
        ..first.clone()
    };
    assert!(provider.build_batch_body(&[first, duplicate_key]).is_err());
}

#[test]
fn typesafe_posts_composite_noul_as_one_request_and_maps_reversed_answers() {
    let mut first =
        JudgmentRequest::noul(encode_text("invoice paid"), "invoice is unpaid").unwrap();
    first.request_key = "r0".into();
    let mut second =
        JudgmentRequest::noul(encode_text("invoice unpaid"), "invoice is unpaid").unwrap();
    second.request_key = "r1".into();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = thread::spawn(move || {
        let response = r#"{"model":"jev-1.13.0","usage":{"input_tokens":100,"output_tokens":10},"answers":{"r1":{"type":"noul","noul":0.98},"r0":{"type":"noul","noul":0.01}}}"#;
        serve_one_request(listener, response)
    });
    let provider = TypesafeProvider::new(
        format!("http://{address}/v1/systemone"),
        "jev-1.13.0".to_string(),
        1_000,
        1_048_576,
        "test-token".to_string(),
    );
    let requests = [first.clone(), second.clone()];
    let result = provider.judge_many(&requests);
    let captured = server.join().unwrap();
    let response = result.expect("verified two-row TypeSafe batch should be dispatched");
    let request = captured.expect("one composite HTTP request should reach the fixture");
    let body = request_body(&request);

    assert_eq!(
        body["state"]["rows"],
        serde_json::json!(["invoice paid", "invoice unpaid"])
    );
    assert_eq!(body["state"]["condition"], "invoice is unpaid");
    assert_eq!(body["questions"].as_object().unwrap().len(), 2);
    assert!(body["questions"]["r0"]["instructions"]
        .as_str()
        .unwrap()
        .contains("state.rows[0]"));
    assert!(body["questions"]["r1"]["instructions"]
        .as_str()
        .unwrap()
        .contains("state.rows[1]"));
    assert!(request
        .to_ascii_lowercase()
        .contains("authorization: bearer test-token"));
    assert_eq!(response.answers.len(), 2);
    assert_eq!(response.answers[0].request_key, "r0");
    assert_eq!(response.answers[0].result, JudgmentResult::Noul(0.01));
    assert_eq!(response.answers[1].request_key, "r1");
    assert_eq!(response.answers[1].result, JudgmentResult::Noul(0.98));
    assert_eq!(
        response.metadata.effective_model.as_deref(),
        Some("jev-1.13.0")
    );
    assert_eq!(response.metadata.usage.unwrap().input_tokens, Some(100));
    assert_eq!(
        provider.estimate_request_bytes(&requests),
        serde_json::to_vec(&body).unwrap().len()
    );
}

#[test]
fn typesafe_rejects_batch_if_the_live_reported_model_has_changed() {
    let mut first = JudgmentRequest::noul(encode_text("invoice paid"), "invoice unpaid").unwrap();
    first.request_key = "r0".into();
    let mut second =
        JudgmentRequest::noul(encode_text("invoice unpaid"), "invoice unpaid").unwrap();
    second.request_key = "r1".into();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = thread::spawn(move || {
        let response = r#"{"model":"jev-1.14.0","answers":{"r0":{"type":"noul","noul":0.1},"r1":{"type":"noul","noul":0.9}}}"#;
        serve_one_request(listener, response)
    });
    let provider = TypesafeProvider::new(
        format!("http://{address}/v1/systemone"),
        "jev-1.13.0".to_string(),
        1_000,
        1_048_576,
        "test-token".to_string(),
    );

    let result = provider.judge_many(&[first, second]);
    let captured = server.join().unwrap();
    assert!(
        captured.is_some(),
        "the composite request reaches the fixture"
    );
    assert!(
        result.is_err(),
        "results from an effective model without live batch evidence fail closed"
    );
}

#[test]
fn typesafe_composite_answers_are_associated_by_key_not_response_order() {
    let provider = provider();
    let mut first = JudgmentRequest::noul(encode_text("paid"), "invoice is paid").unwrap();
    first.request_key = "row_a".into();
    let mut second = JudgmentRequest::choice(
        encode_text("animal is a shark"),
        "which animal?",
        &["bird".into(), "shark".into()],
    )
    .unwrap();
    second.request_key = "row_b".into();

    let parsed = provider
        .parse_batch_response(
            &[first.clone(), second.clone()],
            r#"{"model":"jev-1.13.0","answers":{"row_b":{"type":"choice","choice":"shark"},"row_a":{"type":"noul","noul":0.91}}}"#,
        )
        .unwrap();
    assert_eq!(parsed.answers.len(), 2);
    assert_eq!(parsed.answers[0].request_key, "row_a");
    assert_eq!(parsed.answers[0].result, JudgmentResult::Noul(0.91));
    assert_eq!(parsed.answers[1].request_key, "row_b");
    assert_eq!(
        parsed.answers[1].result,
        JudgmentResult::Choice("shark".into())
    );

    for invalid in [
        r#"{"answers":{"row_a":{"type":"noul","noul":0.5}}}"#,
        r#"{"answers":{"row_a":{"type":"noul","noul":0.5},"row_b":{"type":"choice","choice":"shark"},"extra":{"type":"noul","noul":0.5}}}"#,
        r#"{"answers":{"row_a":{"type":"noul","noul":1.5},"row_b":{"type":"choice","choice":"shark"}}}"#,
        r#"{"answers":{"row_a":{"type":"choice","choice":"bird"},"row_b":{"type":"choice","choice":"shark"}}}"#,
        r#"{"answers":{"row_a":{"type":"noul","noul":0.5},"row_b":{"type":"choice","choice":"whale"}}}"#,
    ] {
        assert!(
            provider
                .parse_batch_response(&[first.clone(), second.clone()], invalid)
                .is_err(),
            "must reject malformed composite response: {invalid}"
        );
    }
}

#[test]
fn localjev_declares_unverified_cross_state_batch_unsupported_and_disables_cache() {
    let provider = LocalJevProvider::new(
        "http://127.0.0.1:1/v1/systemone".into(),
        "nli-deberta-large".into(),
        1_000,
        1_048_576,
        None,
    );
    let capabilities = provider.capabilities();
    assert_eq!(
        capabilities.multi_state_batch,
        CapabilityStatus::Unsupported
    );
    assert_eq!(capabilities.max_choices, Some(255));
    assert_eq!(capabilities.protocol_version, "local-jev/64a0b31");
    assert_eq!(provider.cache_model_identity("jev-1.13.0"), None);
}

#[test]
fn localjev_rejects_choice_over_255_before_network_request() {
    let provider = LocalJevProvider::new(
        "http://127.0.0.1:1/v1/systemone".into(),
        "nli-deberta-large".into(),
        1_000,
        1_048_576,
        None,
    );
    let choices = (0..256)
        .map(|index| format!("choice-{index}"))
        .collect::<Vec<_>>();
    let request = JudgmentRequest::choice(encode_text("synthetic"), "choose", &choices).unwrap();
    let error = provider.judge(&request).unwrap_err();
    assert!(error.message().contains("verified provider limit"));
}

#[test]
fn localjev_uses_its_separate_optional_bearer_credential() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_http_request(&mut stream).to_ascii_lowercase();
        let authorization_seen = request.contains("authorization: bearer private-local-token");
        let body = r#"{"model":"nli-deberta-large","answers":{"r0":{"type":"noul","noul":0.7}}}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
        authorization_seen
    });
    let provider = LocalJevProvider::new(
        format!("http://{address}/v1/systemone"),
        "nli-deberta-large".into(),
        1_000,
        1_048_576,
        Some("private-local-token".into()),
    );
    let request = JudgmentRequest::noul(encode_text("synthetic"), "criterion").unwrap();
    let result = provider.judge(&request);
    assert!(server.join().unwrap());
    assert_eq!(result.unwrap(), JudgmentResult::Noul(0.7));
}

#[test]
fn localjev_truncation_header_is_rejected_without_returning_partial_result() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _request = read_http_request(&mut stream);
        let body = r#"{"model":"nli-deberta-large","answers":{"r0":{"type":"noul","noul":0.9}}}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nx-local-jev-truncated: true\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });
    let provider = LocalJevProvider::new(
        format!("http://{address}/v1/systemone"),
        "nli-deberta-large".into(),
        1_000,
        1_048_576,
        None,
    );
    let request = JudgmentRequest::noul(encode_text("synthetic"), "criterion").unwrap();
    let error = provider.judge(&request).unwrap_err();
    server.join().unwrap();
    assert!(
        error.message().contains("truncated the state"),
        "unexpected local-jev error: {}",
        error.message()
    );
}
