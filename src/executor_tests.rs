#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, Mutex};
    use std::thread;
    use std::time::Duration;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    use super::{
        execute_batch_groups, execute_groups, group_by_identity, ExecutionLimits, JudgmentWork,
        RowTarget,
    };
    use crate::judgment::{
        JudgmentIdentity, JudgmentIdentityContext, JudgmentRequest, JudgmentResult, QuestionKind,
    };
    use crate::provider::{
        CapabilityStatus, Provider, ProviderAnswer, ProviderBatchResult, ProviderCapabilities,
        ProviderMetadata,
    };
    use crate::serialize::encode_text;

    #[derive(Clone, Default)]
    struct RecordingBatchProvider {
        calls: Arc<Mutex<Vec<Vec<String>>>>,
    }

    #[derive(Clone, Default)]
    struct ConcurrentProvider {
        calls: Arc<AtomicUsize>,
        active: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
        fail: bool,
    }

    #[derive(Clone, Copy)]
    enum HttpResponseMode {
        Valid,
        Missing,
        Slow,
    }

    struct HttpBatchProvider {
        url: String,
        agent: ureq::Agent,
    }

    impl HttpBatchProvider {
        fn new(url: String, timeout_ms: u64) -> Self {
            let agent = ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_millis(timeout_ms)))
                .http_status_as_error(false)
                .build()
                .new_agent();
            Self { url, agent }
        }
    }

    fn read_request(stream: &mut TcpStream) -> serde_json::Value {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let count = stream.read(&mut chunk).unwrap_or(0);
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..count]);
            let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            let length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or_default();
            if bytes.len() >= header_end + 4 + length {
                return serde_json::from_slice(&bytes[header_end + 4..header_end + 4 + length])
                    .unwrap();
            }
        }
        panic!("client request ended before full JSON body arrived");
    }

    fn spawn_http_batch_stub(
        expected_requests: usize,
        mode: HttpResponseMode,
    ) -> (String, Arc<AtomicUsize>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&requests);
        let worker = thread::spawn(move || {
            for _ in 0..expected_requests {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_request(&mut stream);
                observed.fetch_add(1, Ordering::SeqCst);
                let questions = request["questions"].as_object().unwrap();
                let rows = request["state"]["rows"].as_array().unwrap();
                if matches!(mode, HttpResponseMode::Slow) {
                    thread::sleep(Duration::from_millis(250));
                }
                let mut entries = Vec::new();
                let mut items = questions.iter().collect::<Vec<_>>();
                items.reverse();
                if matches!(mode, HttpResponseMode::Missing) {
                    items.pop();
                }
                for (key, _question) in items {
                    let row_index = key.trim_start_matches('r').parse::<usize>().unwrap();
                    let row = rows.get(row_index).and_then(serde_json::Value::as_str).unwrap_or("");
                    let value = if row == "low" { 0.2 } else { 0.8 };
                    entries.push(format!("\"{key}\":{{\"type\":\"noul\",\"noul\":{value}}}"));
                }
                let body = format!(
                    "{{\"model\":\"stub-fixed\",\"usage\":{{\"input_tokens\":2,\"output_tokens\":0}},\"answers\":{{{}}}}}",
                    entries.join(",")
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(), body
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (format!("http://{address}/v1/systemone"), requests, worker)
    }

    impl Provider for HttpBatchProvider {
        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities {
                protocol_version: "http-test/v1".into(),
                noul: CapabilityStatus::Verified,
                choice: CapabilityStatus::Verified,
                multi_state_batch: CapabilityStatus::Verified,
                max_batch_judgments: Some(2),
                max_choices: Some(255),
            }
        }
        fn makes_external_request(&self) -> bool { true }
        fn judge(&self, _request: &JudgmentRequest) -> Result<JudgmentResult, crate::judgment::JudgmentError> {
            Err(crate::judgment::JudgmentError::configuration("single request path not used"))
        }
        fn judge_many(&self, requests: &[JudgmentRequest]) -> Result<ProviderBatchResult, crate::judgment::JudgmentError> {
            let rows = requests.iter().map(|request| {
                request.state.as_text().map(str::to_owned).unwrap_or_default()
            }).collect::<Vec<_>>();
            let questions = requests.iter().map(|request| {
                (request.request_key.clone(), serde_json::json!({"type":"noul"}))
            }).collect::<serde_json::Map<_,_>>();
            let body = serde_json::json!({"model":"test-fixed", "state":{"rows":rows}, "questions":questions});
            let mut response = self.agent.post(&self.url).header("Content-Type", "application/json")
                .send_json(&body)
                .map_err(|_| crate::judgment::JudgmentError::provider("local batch stub request failed"))?;
            if !(200..300).contains(&response.status().as_u16()) {
                return Err(crate::judgment::JudgmentError::provider("local batch stub returned an error"));
            }
            let payload: serde_json::Value = response.body_mut().read_json()
                .map_err(|_| crate::judgment::JudgmentError::invalid_response("stub JSON invalid"))?;
            let answers = payload["answers"].as_object()
                .ok_or_else(|| crate::judgment::JudgmentError::invalid_response("answers missing"))?;
            let mut parsed = Vec::new();
            for request in requests {
                let answer = answers.get(&request.request_key)
                    .ok_or_else(|| crate::judgment::JudgmentError::invalid_response("answer key missing"))?;
                if answer["type"].as_str() != Some("noul") {
                    return Err(crate::judgment::JudgmentError::invalid_response("wrong response type"));
                }
                let value = answer["noul"].as_f64()
                    .ok_or_else(|| crate::judgment::JudgmentError::invalid_response("noul value invalid"))?;
                parsed.push(ProviderAnswer {
                    request_key: request.request_key.clone(),
                    result: JudgmentResult::Noul(value),
                });
            }
            Ok(ProviderBatchResult {
                answers: parsed,
                metadata: ProviderMetadata {
                    requested_model: Some("test-fixed".into()),
                    effective_model: payload["model"].as_str().map(str::to_owned),
                    usage: Some(crate::provider::ProviderUsage { input_tokens: Some(2), output_tokens: Some(0) }),
                    ..ProviderMetadata::default()
                },
            })
        }
    }

    impl Provider for ConcurrentProvider {
        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities {
                protocol_version: "concurrent-test/1".into(),
                noul: CapabilityStatus::Verified,
                choice: CapabilityStatus::Verified,
                multi_state_batch: CapabilityStatus::Unsupported,
                max_batch_judgments: None,
                max_choices: None,
            }
        }

        fn judge(
            &self,
            _request: &JudgmentRequest,
        ) -> Result<JudgmentResult, crate::judgment::JudgmentError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(active, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(60));
            self.active.fetch_sub(1, Ordering::SeqCst);
            if self.fail {
                Err(crate::judgment::JudgmentError::provider(
                    "scripted provider failure",
                ))
            } else {
                Ok(JudgmentResult::Noul(0.5))
            }
        }
    }

    impl Provider for RecordingBatchProvider {
        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities {
                protocol_version: "test-batch/1".into(),
                noul: CapabilityStatus::Verified,
                choice: CapabilityStatus::Verified,
                multi_state_batch: CapabilityStatus::Verified,
                max_batch_judgments: Some(2),
                max_choices: None,
            }
        }

        fn judge(
            &self,
            request: &JudgmentRequest,
        ) -> Result<JudgmentResult, crate::judgment::JudgmentError> {
            Ok(match request.kind {
                QuestionKind::Noul => {
                    if request.state.as_text() == Some("low") {
                        JudgmentResult::Noul(0.2)
                    } else {
                        JudgmentResult::Noul(0.8)
                    }
                }
                QuestionKind::Choice => JudgmentResult::Choice(request.choices[0].clone()),
            })
        }

        fn estimate_request_bytes(&self, requests: &[JudgmentRequest]) -> usize {
            requests.len() * 1_000
        }

        fn judge_many(
            &self,
            requests: &[JudgmentRequest],
        ) -> Result<ProviderBatchResult, crate::judgment::JudgmentError> {
            self.calls.lock().unwrap().push(
                requests
                    .iter()
                    .map(|request| request.request_key.clone())
                    .collect(),
            );
            let mut answers = requests
                .iter()
                .map(|request| {
                    Ok(ProviderAnswer {
                        request_key: request.request_key.clone(),
                        result: self.judge(request)?,
                    })
                })
                .collect::<Result<Vec<_>, crate::judgment::JudgmentError>>()?;
            answers.reverse();
            Ok(ProviderBatchResult {
                answers,
                metadata: ProviderMetadata::default(),
            })
        }
    }

    fn item(state: &str, criterion: &str, row: usize) -> JudgmentWork {
        let request = JudgmentRequest::noul(encode_text(state), criterion).unwrap();
        let identity = JudgmentIdentity::new(
            &request,
            JudgmentIdentityContext {
                provider_namespace: "test".into(),
                endpoint: "local".into(),
                requested_model: "fixed".into(),
                effective_model: None,
                credential_scope: 1,
                model_binding_scope: 1,
            },
        );
        JudgmentWork {
            identity,
            request,
            target: RowTarget {
                function_invocation: 1,
                data_chunk: 1,
                row,
            },
        }
    }

    #[test]
    fn batch_executor_deduplicates_maps_reordered_answers_and_splits_limits() {
        let groups = group_by_identity(vec![
            item("low", "is it low?", 0),
            item("high", "is it low?", 1),
            item("low", "is it low?", 2),
        ]);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].targets.len(), 2);

        for (max_judgments, max_bytes, expected_calls) in
            [(16, 10_000, 1), (1, 10_000, 2), (16, 1_500, 2)]
        {
            let runtime = Arc::new(Mutex::new(crate::host::ConnectionRuntime::default()));
            let query_scope = {
                let mut runtime = runtime.lock().unwrap();
                runtime.begin_query();
                runtime.identity_scopes().1
            };
            let provider = RecordingBatchProvider::default();
            let output = execute_batch_groups(
                &groups,
                &provider,
                &crate::host::RuntimeAdapter(&runtime),
                max_judgments,
                ExecutionLimits {
                    query_scope,
                    max_inflight: 4,
                    max_request_bytes: max_bytes,
                },
                None,
            )
            .unwrap();
            assert_eq!(provider.calls.lock().unwrap().len(), expected_calls);
            assert_eq!(output.len(), 3);
            let by_row: std::collections::HashMap<_, _> = output
                .into_iter()
                .map(|(target, result)| (target.row, result))
                .collect();
            assert_eq!(by_row[&0], JudgmentResult::Noul(0.2));
            assert_eq!(by_row[&1], JudgmentResult::Noul(0.8));
            assert_eq!(by_row[&2], JudgmentResult::Noul(0.2));
        }
    }

    #[test]
    fn batch_executor_respects_provider_limit_and_separates_question_contexts() {
        let groups = group_by_identity(vec![
            item("low", "question A", 0),
            item("high", "question A", 1),
            item("middle", "question A", 2),
            item("other", "question B", 3),
        ]);
        let runtime = Arc::new(Mutex::new(crate::host::ConnectionRuntime::default()));
        let query_scope = {
            let mut runtime = runtime.lock().unwrap();
            runtime.begin_query();
            runtime.identity_scopes().1
        };
        let provider = RecordingBatchProvider::default();
        let output = execute_batch_groups(
            &groups,
            &provider,
            &crate::host::RuntimeAdapter(&runtime),
            16,
            ExecutionLimits {
                query_scope,
                max_inflight: 4,
                max_request_bytes: 10_000,
            },
            None,
        )
        .unwrap();

        let calls = provider.calls.lock().unwrap();
        assert_eq!(
            calls.len(),
            3,
            "provider cap splits A while question B stays separate"
        );
        assert!(calls.iter().all(|keys| keys.len() <= 2));
        assert_eq!(output.len(), 4);
    }

    #[test]
    fn controlled_http_batch_uses_one_roundtrip_for_two_judgments_and_splits_over_limit() {
        let (url, attempts, server) = spawn_http_batch_stub(2, HttpResponseMode::Valid);
        let provider = HttpBatchProvider::new(url, 2_000);
        let groups = group_by_identity(vec![
            item("low", "same criterion", 0),
            item("high", "same criterion", 1),
            item("middle", "same criterion", 2),
            item("other", "same criterion", 3),
        ]);
        let runtime = Arc::new(Mutex::new(crate::host::ConnectionRuntime::default()));
        let query_scope = {
            let mut runtime = runtime.lock().unwrap();
            runtime.begin_query();
            let query_scope = runtime.identity_scopes().1;
            runtime.record_optimized_rows(query_scope, 4, 0, 4, 4);
            query_scope
        };
        let output = execute_batch_groups(
            &groups,
            &provider,
            &crate::host::RuntimeAdapter(&runtime),
            16,
            ExecutionLimits {
                query_scope,
                max_inflight: 2,
                max_request_bytes: 1_000_000,
            },
            None,
        )
        .unwrap();
        server.join().unwrap();
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert_eq!(output.len(), 4);
        let by_row: std::collections::HashMap<_, _> = output
            .into_iter()
            .map(|(target, result)| (target.row, result))
            .collect();
        assert_eq!(by_row[&0], JudgmentResult::Noul(0.2));
        assert_eq!(by_row[&1], JudgmentResult::Noul(0.8));
        assert_eq!(by_row[&2], JudgmentResult::Noul(0.8));
        assert_eq!(by_row[&3], JudgmentResult::Noul(0.8));
        runtime
            .lock()
            .unwrap()
            .end_query(0);
        let guard = runtime.lock().unwrap();
        let counts = &guard.last_query().unwrap().counts;
        assert_eq!(counts.external_requests, 2);
        assert_eq!(counts.batched_requests, 2);
        assert_eq!(counts.batch_size_histogram.get(&2), Some(&2));
    }

    #[test]
    fn controlled_http_batch_missing_answer_fails_once_without_retry() {
        let (url, attempts, server) = spawn_http_batch_stub(1, HttpResponseMode::Missing);
        let provider = HttpBatchProvider::new(url, 2_000);
        let groups = group_by_identity(vec![
            item("low", "same criterion", 0),
            item("high", "same criterion", 1),
        ]);
        let runtime = Arc::new(Mutex::new(crate::host::ConnectionRuntime::default()));
        let query_scope = {
            let mut runtime = runtime.lock().unwrap();
            runtime.begin_query();
            let query_scope = runtime.identity_scopes().1;
            runtime.record_optimized_rows(query_scope, 2, 0, 2, 2);
            query_scope
        };
        assert!(execute_batch_groups(
            &groups,
            &provider,
            &crate::host::RuntimeAdapter(&runtime),
            16,
            ExecutionLimits {
                query_scope,
                max_inflight: 2,
                max_request_bytes: 1_000_000,
            },
            None,
        )
        .is_err());
        server.join().unwrap();
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        runtime
            .lock()
            .unwrap()
            .end_query(duckdb::ffi::duckdb_error_type_DUCKDB_ERROR_INVALID_INPUT);
        let guard = runtime.lock().unwrap();
        let counts = &guard.last_query().unwrap().counts;
        assert_eq!(counts.external_requests, 1);
        assert_eq!(counts.failed_requests, 1);
    }

    #[test]
    fn controlled_http_batch_timeout_is_counted_once_and_not_retried() {
        let (url, attempts, server) = spawn_http_batch_stub(1, HttpResponseMode::Slow);
        let provider = HttpBatchProvider::new(url, 50);
        let groups = group_by_identity(vec![
            item("low", "same criterion", 0),
            item("high", "same criterion", 1),
        ]);
        let runtime = Arc::new(Mutex::new(crate::host::ConnectionRuntime::default()));
        let query_scope = {
            let mut runtime = runtime.lock().unwrap();
            runtime.begin_query();
            let query_scope = runtime.identity_scopes().1;
            runtime.record_optimized_rows(query_scope, 2, 0, 2, 2);
            query_scope
        };
        assert!(execute_batch_groups(
            &groups,
            &provider,
            &crate::host::RuntimeAdapter(&runtime),
            16,
            ExecutionLimits {
                query_scope,
                max_inflight: 2,
                max_request_bytes: 1_000_000,
            },
            None,
        )
        .is_err());
        server.join().unwrap();
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        runtime
            .lock()
            .unwrap()
            .end_query(duckdb::ffi::duckdb_error_type_DUCKDB_ERROR_INVALID_INPUT);
        let guard = runtime.lock().unwrap();
        let counts = &guard.last_query().unwrap().counts;
        assert_eq!(counts.external_requests, 1);
        assert_eq!(counts.failed_requests, 1);
    }

    #[test]
    fn query_executor_shares_inflight_identity_and_obeys_connection_limit() {
        fn begin_runtime() -> (crate::host::SharedConnectionRuntime, u64) {
            let runtime = Arc::new(Mutex::new(crate::host::ConnectionRuntime::default()));
            let query_scope = {
                let mut guard = runtime.lock().unwrap();
                guard.begin_query();
                guard.identity_scopes().1
            };
            (runtime, query_scope)
        }

        let (runtime, query_scope) = begin_runtime();
        let provider = ConcurrentProvider::default();
        let groups = (0..4)
            .map(|row| group_by_identity(vec![item("state", &format!("criterion {row}"), row)]))
            .collect::<Vec<_>>();
        let barrier = Arc::new(Barrier::new(groups.len()));
        let workers = groups
            .into_iter()
            .map(|group| {
                let provider = provider.clone();
                let runtime = Arc::clone(&runtime);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    execute_groups(
                        &group,
                        &provider,
                        &crate::host::RuntimeAdapter(&runtime),
                        ExecutionLimits {
                            query_scope,
                            max_inflight: 2,
                            max_request_bytes: 1_000_000,
                        },
                        None,
                    )
                    .unwrap()
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            assert_eq!(worker.join().unwrap().len(), 1);
        }
        assert_eq!(provider.calls.load(Ordering::SeqCst), 4);
        assert_eq!(provider.peak.load(Ordering::SeqCst), 2);
        assert_eq!(runtime.lock().unwrap().peak_inflight(), 2);

        let (runtime, query_scope) = begin_runtime();
        let duplicate_group = group_by_identity(vec![item("same", "same question", 0)])[0].clone();
        let provider = ConcurrentProvider::default();
        let barrier = Arc::new(Barrier::new(2));
        let workers = (0..2)
            .map(|_| {
                let provider = provider.clone();
                let runtime = Arc::clone(&runtime);
                let barrier = Arc::clone(&barrier);
                let group = duplicate_group.clone();
                thread::spawn(move || {
                    barrier.wait();
                    execute_groups(
                        std::slice::from_ref(&group),
                        &provider,
                        &crate::host::RuntimeAdapter(&runtime),
                        ExecutionLimits {
                            query_scope,
                            max_inflight: 1,
                            max_request_bytes: 1_000_000,
                        },
                        None,
                    )
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            assert_eq!(worker.join().unwrap().unwrap().len(), 1);
        }
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);

        let (runtime, query_scope) = begin_runtime();
        let provider = ConcurrentProvider {
            fail: true,
            ..ConcurrentProvider::default()
        };
        let barrier = Arc::new(Barrier::new(2));
        let workers = (0..2)
            .map(|_| {
                let provider = provider.clone();
                let runtime = Arc::clone(&runtime);
                let barrier = Arc::clone(&barrier);
                let group = duplicate_group.clone();
                thread::spawn(move || {
                    barrier.wait();
                    execute_groups(
                        std::slice::from_ref(&group),
                        &provider,
                        &crate::host::RuntimeAdapter(&runtime),
                        ExecutionLimits {
                            query_scope,
                            max_inflight: 1,
                            max_request_bytes: 1_000_000,
                        },
                        None,
                    )
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            assert!(worker.join().unwrap().is_err());
        }
        assert_eq!(
            provider.calls.load(Ordering::SeqCst),
            1,
            "failed work is not retried"
        );
        let another_identity = group_by_identity(vec![item("other", "another question", 1)]);
        assert!(
            execute_groups(
                &another_identity,
                &provider,
                &crate::host::RuntimeAdapter(&runtime),
                ExecutionLimits {
                    query_scope,
                    max_inflight: 1,
                    max_request_bytes: 1_000_000,
                },
                None,
            )
            .is_err()
        );
        assert_eq!(
            provider.calls.load(Ordering::SeqCst),
            2,
            "the permit is returned after a provider failure"
        );
    }
}
