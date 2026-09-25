//! Query-local identity grouping, provider execution, and output row association.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use crate::cache::CachePolicy;
use crate::judgment::{
    validate_judgment_result, JudgmentError, JudgmentIdentity, JudgmentRequest, JudgmentResult,
};
use crate::provider::{validate_batch_result, CapabilityStatus, Provider};
use crate::runtime::{ExecutionRuntime, QueryEntry};

/// Identifies one scalar invocation's output location. Row numbers alone are not query-unique.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RowTarget {
    pub function_invocation: u64,
    pub data_chunk: u64,
    pub row: usize,
}

#[derive(Debug, Clone)]
pub struct JudgmentWork {
    pub identity: JudgmentIdentity,
    pub request: JudgmentRequest,
    pub target: RowTarget,
}

#[derive(Debug, Clone)]
pub struct JudgmentGroup {
    pub identity: JudgmentIdentity,
    pub request: JudgmentRequest,
    pub targets: Vec<RowTarget>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionLimits {
    pub query_scope: u64,
    pub max_inflight: usize,
    pub max_request_bytes: usize,
}

type IndexedBatchResults = Vec<(usize, JudgmentResult)>;
type BatchAttemptResult = Result<(IndexedBatchResults, Option<String>), JudgmentError>;
type BatchFollowers = Vec<(usize, Arc<QueryEntry>, Arc<std::sync::atomic::AtomicBool>)>;

/// Groups identical judgments while retaining every input row target.
/// The HashMap compares the full `JudgmentIdentity` after using its hash for lookup.
pub fn group_by_identity(work: Vec<JudgmentWork>) -> Vec<JudgmentGroup> {
    let mut groups: Vec<JudgmentGroup> = Vec::new();
    let mut positions: HashMap<JudgmentIdentity, usize> = HashMap::new();

    for item in work {
        if let Some(index) = positions.get(&item.identity).copied() {
            groups[index].targets.push(item.target);
        } else {
            let index = groups.len();
            positions.insert(item.identity.clone(), index);
            groups.push(JudgmentGroup {
                identity: item.identity,
                request: item.request,
                targets: vec![item.target],
            });
        }
    }
    groups
}

/// Executes each group once, shares its in-flight/completed entry across scalar instances in this
/// query, and expands the result back to every target.
pub fn execute_groups(
    groups: &[JudgmentGroup],
    provider: &dyn Provider,
    runtime: &dyn ExecutionRuntime,
    limits: ExecutionLimits,
    cache_policy: Option<&CachePolicy>,
) -> Result<Vec<(RowTarget, JudgmentResult)>, JudgmentError> {
    if limits.max_request_bytes == 0 {
        return Err(JudgmentError::configuration(
            "duckjeu_max_request_bytes must be positive",
        ));
    }
    let mut output = Vec::new();
    for group in groups {
        let result = execute_one_group(group, provider, runtime, limits, cache_policy)?;
        output.extend(
            group
                .targets
                .iter()
                .copied()
                .map(|target| (target, result.clone())),
        );
    }
    Ok(output)
}

fn execute_one_group(
    group: &JudgmentGroup,
    provider: &dyn Provider,
    runtime: &dyn ExecutionRuntime,
    limits: ExecutionLimits,
    cache_policy: Option<&CachePolicy>,
) -> Result<JudgmentResult, JudgmentError> {
    let cache_identity = cache_policy.and_then(|_| cache_identity_for(group, provider));
    let (entry, owns_request) = runtime.claim_query_entry(limits.query_scope, &group.identity)?;
    let cancelled = runtime.query_cancellation(limits.query_scope)?;
    if !owns_request {
        return entry.wait(&cancelled);
    }

    if let Some(policy) = cache_policy {
        if let Err(error) = policy.validate() {
            entry.complete(Err(error.clone()));
            return Err(error);
        }
        let cached = runtime.cache_lookup(cache_identity.as_ref(), Instant::now(), policy);
        if let Some(result) = cached {
            runtime.record_cache_hit(limits.query_scope);
            entry.complete(Ok(result.clone()));
            return Ok(result);
        }
    }

    if provider.estimate_request_bytes(std::slice::from_ref(&group.request))
        > limits.max_request_bytes
    {
        let error = JudgmentError::configuration("one judgment exceeds duckjeu_max_request_bytes");
        entry.complete(Err(error.clone()));
        return Err(error);
    }

    let external = provider.makes_external_request();
    let mut attempted = false;
    let mut response_received = false;
    let mut provider_elapsed = std::time::Duration::ZERO;
    let result = (|| {
        let _permit = runtime.acquire_permit(limits.query_scope, limits.max_inflight)?;
        runtime.record_provider_attempt(limits.query_scope, 1, external);
        attempted = true;
        let started = Instant::now();
        let response = provider.judge_with_metadata(&group.request);
        provider_elapsed = started.elapsed();
        let response = response?;
        runtime.record_provider_response(
            limits.query_scope,
            &response.metadata,
            provider_elapsed,
            external,
        );
        response_received = true;
        validate_batch_result(std::slice::from_ref(&group.request), &response)?;
        let effective_model = response.metadata.effective_model.clone();
        let result = response
            .answers
            .into_iter()
            .next()
            .ok_or_else(|| JudgmentError::invalid_response("provider returned no answer"))?
            .result;
        validate_judgment_result(&group.request, &result)?;
        if let (Some(cache_identity), Some(policy)) = (&cache_identity, cache_policy) {
            if effective_model.as_deref() == cache_identity.effective_model.as_deref()
                && provider_model_is_stable(provider, group, &cache_identity.effective_model)
            {
                runtime.cache_insert(
                    cache_identity.clone(),
                    result.clone(),
                    Instant::now(),
                    policy,
                );
            }
        }
        Ok(result)
    })();
    if attempted && result.is_err() && external {
        runtime.record_failed_request(limits.query_scope);
        if !response_received {
            runtime.record_provider_failure(limits.query_scope, provider_elapsed, external);
        }
    }
    entry.complete(result.clone());
    result
}

fn cache_identity_for(group: &JudgmentGroup, provider: &dyn Provider) -> Option<JudgmentIdentity> {
    let effective_model = provider.cache_model_identity(&group.identity.requested_model)?;
    let mut identity = group.identity.clone();
    identity.effective_model = Some(effective_model);
    identity.unresolved_model_scope = None;
    Some(identity)
}

fn provider_model_is_stable(
    provider: &dyn Provider,
    group: &JudgmentGroup,
    expected_model: &Option<String>,
) -> bool {
    expected_model.as_deref().is_some_and(|expected| {
        provider
            .cache_model_identity(&group.identity.requested_model)
            .as_deref()
            == Some(expected)
    })
}

/// Batches distinct compatible identities, with exact request-key validation before expansion.
/// A provider must declare verified multi-state support before the first call is made.
pub fn execute_batch_groups(
    groups: &[JudgmentGroup],
    provider: &dyn Provider,
    runtime: &dyn ExecutionRuntime,
    max_judgments: usize,
    limits: ExecutionLimits,
    cache_policy: Option<&CachePolicy>,
) -> Result<Vec<(RowTarget, JudgmentResult)>, JudgmentError> {
    let capabilities = provider.capabilities();
    if capabilities.multi_state_batch != CapabilityStatus::Verified {
        return Err(JudgmentError::configuration(
            "provider multi-state batch capability is not verified",
        ));
    }
    if max_judgments == 0 || limits.max_request_bytes == 0 {
        return Err(JudgmentError::configuration(
            "batch judgment and request byte limits must be positive",
        ));
    }
    let max_judgments = capabilities
        .max_batch_judgments
        .map_or(max_judgments, |service_limit| {
            max_judgments.min(service_limit)
        });
    if max_judgments == 0 {
        return Err(JudgmentError::configuration(
            "provider advertises a zero judgment batch limit",
        ));
    }

    let mut output = Vec::new();
    let mut compatible_partitions: Vec<Vec<usize>> = Vec::new();
    let mut leaders: HashMap<usize, Arc<QueryEntry>> = HashMap::new();
    let mut cache_keys: HashMap<usize, JudgmentIdentity> = HashMap::new();
    let mut followers: BatchFollowers = Vec::new();

    for (index, group) in groups.iter().enumerate() {
        if capabilities.supports(group.request.kind) != CapabilityStatus::Verified {
            return Err(JudgmentError::configuration(
                "provider capability for this question type is not verified",
            ));
        }
        if capabilities
            .max_choices
            .is_some_and(|limit| group.request.choices.len() > limit)
        {
            return Err(JudgmentError::configuration(
                "choice count exceeds the verified provider limit",
            ));
        }
        let cache_identity = cache_policy.and_then(|_| cache_identity_for(group, provider));
        let (entry, owns_request) =
            runtime.claim_query_entry(limits.query_scope, &group.identity)?;
        let cancelled = runtime.query_cancellation(limits.query_scope)?;
        if owns_request {
            if let Some(policy) = cache_policy {
                if let Err(error) = policy.validate() {
                    entry.complete(Err(error.clone()));
                    return Err(error);
                }
                let cached = runtime.cache_lookup(cache_identity.as_ref(), Instant::now(), policy);
                if let Some(result) = cached {
                    runtime.record_cache_hit(limits.query_scope);
                    entry.complete(Ok(result.clone()));
                    output.extend(
                        group
                            .targets
                            .iter()
                            .copied()
                            .map(|target| (target, result.clone())),
                    );
                    continue;
                }
            }
            leaders.insert(index, entry);
            if let Some(cache_identity) = cache_identity {
                cache_keys.insert(index, cache_identity);
            }
        } else {
            followers.push((index, entry, cancelled));
            continue;
        }
        if let Some(partition) = compatible_partitions.iter_mut().find(|partition| {
            groups[partition[0]]
                .identity
                .same_batch_context(&group.identity)
        }) {
            partition.push(index);
        } else {
            compatible_partitions.push(vec![index]);
        }
    }

    for partition in compatible_partitions {
        let mut current = Vec::new();
        for index in partition {
            if !leaders.contains_key(&index) {
                continue;
            }
            let candidate = requests_for_groups(groups, &current, Some(index));
            let exceeds_count = candidate.len() > max_judgments;
            let exceeds_bytes =
                provider.estimate_request_bytes(&candidate) > limits.max_request_bytes;
            if exceeds_count || exceeds_bytes {
                if current.is_empty() {
                    let error = JudgmentError::configuration(
                        "one judgment exceeds duckjeu_max_request_bytes",
                    );
                    fail_leaders(&leaders, &error);
                    return Err(error);
                }
                let dispatch = BatchDispatch {
                    groups,
                    leaders: &leaders,
                    cache_keys: &cache_keys,
                    provider,
                    runtime,
                    limits,
                    cache_policy,
                };
                if let Err(error) = dispatch.dispatch(&current, &mut output) {
                    fail_leaders(&leaders, &error);
                    return Err(error);
                }
                current.clear();
                let singleton = requests_for_groups(groups, &current, Some(index));
                if singleton.len() > max_judgments
                    || provider.estimate_request_bytes(&singleton) > limits.max_request_bytes
                {
                    let error = JudgmentError::configuration(
                        "one judgment exceeds the configured batch limits",
                    );
                    fail_leaders(&leaders, &error);
                    return Err(error);
                }
                current.push(index);
            } else {
                current.push(index);
            }
        }
        if !current.is_empty() {
            let dispatch = BatchDispatch {
                groups,
                leaders: &leaders,
                cache_keys: &cache_keys,
                provider,
                runtime,
                limits,
                cache_policy,
            };
            if let Err(error) = dispatch.dispatch(&current, &mut output) {
                fail_leaders(&leaders, &error);
                return Err(error);
            }
        }
    }

    for (index, entry, cancelled) in followers {
        let result = entry.wait(&cancelled)?;
        output.extend(
            groups[index]
                .targets
                .iter()
                .copied()
                .map(|target| (target, result.clone())),
        );
    }
    Ok(output)
}

fn fail_leaders(leaders: &HashMap<usize, Arc<QueryEntry>>, error: &JudgmentError) {
    for entry in leaders.values() {
        entry.complete(Err(error.clone()));
    }
}

fn requests_for_groups(
    groups: &[JudgmentGroup],
    indices: &[usize],
    additional: Option<usize>,
) -> Vec<JudgmentRequest> {
    let mut all = indices.to_vec();
    if let Some(index) = additional {
        all.push(index);
    }
    all.into_iter()
        .enumerate()
        .map(|(batch_index, group_index)| {
            let mut request = groups[group_index].request.clone();
            request.request_key = format!("r{batch_index}");
            request
        })
        .collect()
}

#[derive(Clone, Copy)]
struct BatchDispatch<'a> {
    groups: &'a [JudgmentGroup],
    leaders: &'a HashMap<usize, Arc<QueryEntry>>,
    cache_keys: &'a HashMap<usize, JudgmentIdentity>,
    provider: &'a dyn Provider,
    runtime: &'a dyn ExecutionRuntime,
    limits: ExecutionLimits,
    cache_policy: Option<&'a CachePolicy>,
}

impl BatchDispatch<'_> {
    fn dispatch(
        &self,
        indices: &[usize],
        output: &mut Vec<(RowTarget, JudgmentResult)>,
    ) -> Result<(), JudgmentError> {
        let Self {
            groups,
            leaders,
            cache_keys,
            provider,
            runtime,
            limits,
            cache_policy,
        } = *self;
        let requests = requests_for_groups(groups, indices, None);
        let external = provider.makes_external_request();
        let mut attempted = false;
        let mut response_received = false;
        let mut provider_elapsed = std::time::Duration::ZERO;
        let attempt: BatchAttemptResult = (|| {
            let _permit = runtime.acquire_permit(limits.query_scope, limits.max_inflight)?;
            runtime.record_provider_attempt(limits.query_scope, requests.len(), external);
            attempted = true;
            let started = Instant::now();
            let response = provider.judge_many(&requests);
            provider_elapsed = started.elapsed();
            let response = response?;
            runtime.record_provider_response(
                limits.query_scope,
                &response.metadata,
                provider_elapsed,
                external,
            );
            response_received = true;
            validate_batch_result(&requests, &response)?;
            let effective_model = response.metadata.effective_model.clone();
            let answers: HashMap<_, _> = response
                .answers
                .into_iter()
                .map(|answer| (answer.request_key, answer.result))
                .collect();
            let mut results = Vec::with_capacity(requests.len());
            for (index, request) in indices.iter().zip(requests.iter()) {
                let result = answers.get(&request.request_key).ok_or_else(|| {
                    JudgmentError::invalid_response(
                        "provider batch answer key disappeared after validation",
                    )
                })?;
                validate_judgment_result(request, result)?;
                results.push((*index, result.clone()));
            }
            Ok((results, effective_model))
        })();
        if attempted && attempt.is_err() && external {
            runtime.record_failed_request(limits.query_scope);
            if !response_received {
                runtime.record_provider_failure(limits.query_scope, provider_elapsed, external);
            }
        }

        match attempt {
            Ok((results, effective_model)) => {
                for (index, result) in results {
                    if let (Some(cache_identity), Some(policy)) =
                        (cache_keys.get(&index), cache_policy)
                    {
                        if effective_model.as_deref() == cache_identity.effective_model.as_deref()
                            && provider_model_is_stable(
                                provider,
                                &groups[index],
                                &cache_identity.effective_model,
                            )
                        {
                            runtime.cache_insert(
                                cache_identity.clone(),
                                result.clone(),
                                Instant::now(),
                                policy,
                            );
                        }
                    }
                    leaders[&index].complete(Ok(result.clone()));
                    output.extend(
                        groups[index]
                            .targets
                            .iter()
                            .copied()
                            .map(|target| (target, result.clone())),
                    );
                }
                Ok(())
            }
            Err(error) => {
                for index in indices {
                    leaders[index].complete(Err(error.clone()));
                }
                Err(error)
            }
        }
    }
}
