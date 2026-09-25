//! Thin, version-locked bridge between DuckDB's ClientContext lifecycle and Rust state.
//!
//! The C++ shim only attaches/releases the runtime state and forwards query lifecycle events.
//! Judgment, provider, cache, and observation rules stay in Rust.

use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::BuildHasher;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[cfg(duckjeu_client_context_bridge)]
use std::ffi::c_void;
#[cfg(duckjeu_client_context_bridge)]
use std::panic::{catch_unwind, AssertUnwindSafe};
#[cfg(duckjeu_client_context_bridge)]
use std::sync::MutexGuard;

use duckdb::ffi::*;

use crate::cache::JudgmentCache;
use crate::judgment::{JudgmentError, JudgmentIdentity, JudgmentResult};
use duckjeu_judgment_core::metrics::QueryCounts;
use duckjeu_judgment_core::provider::ProviderMetadata;
use duckjeu_judgment_core::runtime::{ExecutionRuntime, QueryEntry};

pub mod connection;

pub type SharedConnectionRuntime = Arc<Mutex<ConnectionRuntime>>;

/// Local newtype that injects DuckDB-owned state into the host-independent executor interface.
pub struct RuntimeAdapter<'a>(pub &'a SharedConnectionRuntime);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryOutcome {
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct QuerySnapshot {
    pub query_id: u64,
    pub outcome: QueryOutcome,
    pub elapsed: Duration,
    pub counts: QueryCounts,
    pub provider: Option<String>,
    pub requested_model: Option<String>,
    pub detailed_timing: bool,
}

struct ActiveQuery {
    query_id: u64,
    #[cfg(any(duckjeu_client_context_bridge, test))]
    started_at: Instant,
    cancelled: Arc<AtomicBool>,
    counts: QueryCounts,
    provider: Option<String>,
    requested_model: Option<String>,
    detailed_timing: bool,
}

#[derive(Default)]
struct PermitState {
    active: usize,
    peak: usize,
}

#[derive(Default)]
struct InflightPermitPool {
    state: Mutex<PermitState>,
    changed: Condvar,
}

pub struct InflightPermit {
    pool: Arc<InflightPermitPool>,
}

impl Drop for InflightPermit {
    fn drop(&mut self) {
        let mut state = self
            .pool
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.active = state.active.saturating_sub(1);
        self.pool.changed.notify_all();
    }
}

impl InflightPermitPool {
    fn acquire(
        self: &Arc<Self>,
        max_inflight: usize,
        cancelled: &AtomicBool,
    ) -> Result<InflightPermit, JudgmentError> {
        if max_inflight == 0 {
            return Err(JudgmentError::configuration(
                "duckjeu_max_inflight must be positive",
            ));
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            if cancelled.load(Ordering::Acquire) {
                return Err(JudgmentError::provider(
                    "query was cancelled while waiting for an external request slot",
                ));
            }
            if state.active < max_inflight {
                state.active += 1;
                state.peak = state.peak.max(state.active);
                return Ok(InflightPermit {
                    pool: Arc::clone(self),
                });
            }
            let (next, _) = self
                .changed
                .wait_timeout(state, Duration::from_millis(20))
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state = next;
        }
    }

    #[cfg(any(duckjeu_client_context_bridge, test))]
    fn notify_waiters(&self) {
        self.changed.notify_all();
    }
}

static NEXT_CONNECTION_SCOPE: AtomicU64 = AtomicU64::new(1);

/// Mutable state whose lifetime is bounded by one DuckDB client connection.
pub struct ConnectionRuntime {
    connection_scope: u64,
    credential_scope: u64,
    credential_fingerprint: Option<u64>,
    credential_hasher: RandomState,
    next_query_id: u64,
    active_query: Option<ActiveQuery>,
    last_query: Option<QuerySnapshot>,
    query_entries: HashMap<JudgmentIdentity, Arc<QueryEntry>>,
    inflight: Arc<InflightPermitPool>,
    cache: JudgmentCache,
}

impl Default for ConnectionRuntime {
    fn default() -> Self {
        let connection_scope = NEXT_CONNECTION_SCOPE.fetch_add(1, Ordering::Relaxed);
        Self {
            connection_scope,
            credential_scope: connection_scope,
            credential_fingerprint: None,
            credential_hasher: RandomState::new(),
            next_query_id: 0,
            active_query: None,
            last_query: None,
            query_entries: HashMap::new(),
            inflight: Arc::new(InflightPermitPool::default()),
            cache: JudgmentCache::default(),
        }
    }
}

impl ConnectionRuntime {
    #[cfg(any(duckjeu_client_context_bridge, test))]
    pub(crate) fn begin_query(&mut self) {
        if let Some(previous) = self.active_query.take() {
            previous.cancelled.store(true, Ordering::Release);
            self.inflight.notify_waiters();
        }
        let query_id = self.next_query_id;
        self.next_query_id = self.next_query_id.wrapping_add(1);
        self.query_entries.clear();
        self.active_query = Some(ActiveQuery {
            query_id,
            #[cfg(any(duckjeu_client_context_bridge, test))]
            started_at: Instant::now(),
            cancelled: Arc::new(AtomicBool::new(false)),
            counts: QueryCounts::default(),
            provider: None,
            requested_model: None,
            detailed_timing: false,
        });
    }

    #[cfg(any(duckjeu_client_context_bridge, test))]
    pub(crate) fn end_query(&mut self, error_type: u32) {
        let Some(active) = self.active_query.take() else {
            return;
        };
        active.cancelled.store(true, Ordering::Release);
        self.query_entries.clear();
        self.inflight.notify_waiters();
        let outcome = if error_type == duckdb_error_type_DUCKDB_ERROR_INTERRUPT {
            QueryOutcome::Cancelled
        } else if error_type == 0 {
            QueryOutcome::Succeeded
        } else {
            QueryOutcome::Failed
        };
        if active.counts.input_rows > 0 {
            self.last_query = Some(QuerySnapshot {
                query_id: active.query_id,
                outcome,
                elapsed: active.started_at.elapsed(),
                counts: active.counts,
                provider: active.provider,
                requested_model: active.requested_model,
                detailed_timing: active.detailed_timing,
            });
        }
    }

    pub fn last_query(&self) -> Option<&QuerySnapshot> {
        self.last_query.as_ref()
    }

    pub fn record_profile_context(
        &mut self,
        query_scope: u64,
        provider: &str,
        requested_model: &str,
        detailed_timing: bool,
    ) {
        if let Some(query) = self
            .active_query
            .as_mut()
            .filter(|query| query.query_id == query_scope)
        {
            query.provider.get_or_insert_with(|| provider.to_string());
            query
                .requested_model
                .get_or_insert_with(|| requested_model.to_string());
            query.detailed_timing |= detailed_timing;
        }
    }

    pub fn record_optimized_rows(
        &mut self,
        query_scope: u64,
        input_rows: usize,
        null_rows: usize,
        valid_rows: usize,
        unique_judgments: usize,
    ) {
        let Some(query) = self
            .active_query
            .as_mut()
            .filter(|query| query.query_id == query_scope)
        else {
            return;
        };
        let counts = &mut query.counts;
        counts.input_rows = counts.input_rows.saturating_add(input_rows as u64);
        counts.null_rows = counts.null_rows.saturating_add(null_rows as u64);
        counts.valid_rows = counts.valid_rows.saturating_add(valid_rows as u64);
        counts.deduplicated_rows = counts
            .deduplicated_rows
            .saturating_add(valid_rows.saturating_sub(unique_judgments) as u64);
    }

    pub fn record_row_input(&mut self, query_scope: u64) {
        if let Some(query) = self
            .active_query
            .as_mut()
            .filter(|query| query.query_id == query_scope)
        {
            query.counts.input_rows = query.counts.input_rows.saturating_add(1);
        }
    }

    pub fn record_row_result(&mut self, query_scope: u64, is_null: bool) {
        if let Some(query) = self
            .active_query
            .as_mut()
            .filter(|query| query.query_id == query_scope)
        {
            if is_null {
                query.counts.null_rows = query.counts.null_rows.saturating_add(1);
            } else {
                query.counts.valid_rows = query.counts.valid_rows.saturating_add(1);
                query.counts.unique_judgments = query.counts.unique_judgments.saturating_add(1);
            }
        }
    }

    pub fn record_cache_hit(&mut self, query_scope: u64) {
        if let Some(query) = self
            .active_query
            .as_mut()
            .filter(|query| query.query_id == query_scope)
        {
            query.counts.cache_hit_judgments = query.counts.cache_hit_judgments.saturating_add(1);
        }
    }

    pub fn record_provider_attempt(&mut self, query_scope: u64, batch_size: usize, external: bool) {
        if let Some(query) = self
            .active_query
            .as_mut()
            .filter(|query| query.query_id == query_scope)
        {
            let counts = &mut query.counts;
            counts.provider_invocations = counts.provider_invocations.saturating_add(1);
            if external {
                counts.external_requests = counts.external_requests.saturating_add(1);
                let count = counts.batch_size_histogram.entry(batch_size).or_default();
                *count = count.saturating_add(1);
                if batch_size > 1 {
                    counts.batched_requests = counts.batched_requests.saturating_add(1);
                }
            }
        }
    }

    pub fn record_provider_response(
        &mut self,
        query_scope: u64,
        metadata: &ProviderMetadata,
        elapsed: Duration,
        external: bool,
    ) {
        let Some(query) = self
            .active_query
            .as_mut()
            .filter(|query| query.query_id == query_scope)
        else {
            return;
        };
        if !external {
            return;
        }
        if query.detailed_timing {
            query.counts.external_roundtrip_ns = query.counts.external_roundtrip_ns.saturating_add(
                metadata
                    .external_roundtrip_ns
                    .map(u128::from)
                    .unwrap_or_else(|| elapsed.as_nanos()),
            );
            query.counts.external_roundtrip_count =
                query.counts.external_roundtrip_count.saturating_add(1);
            if let Some(serialization_time_ns) = metadata.serialization_time_ns {
                query.counts.serialization_time_ns = query
                    .counts
                    .serialization_time_ns
                    .saturating_add(serialization_time_ns as u128);
                query.counts.serialization_observed_count =
                    query.counts.serialization_observed_count.saturating_add(1);
            }
        }
        if let Some(model) = metadata
            .effective_model
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            query.counts.reported_models.insert(model.to_string());
        }
        query.counts.usage_response_count = query.counts.usage_response_count.saturating_add(1);
        match metadata.usage.as_ref() {
            Some(usage) => match (usage.input_tokens, usage.output_tokens) {
                (Some(input), Some(output)) => {
                    query.counts.input_tokens = Some(
                        query
                            .counts
                            .input_tokens
                            .unwrap_or_default()
                            .saturating_add(input),
                    );
                    query.counts.output_tokens = Some(
                        query
                            .counts
                            .output_tokens
                            .unwrap_or_default()
                            .saturating_add(output),
                    );
                }
                _ => {
                    query.counts.usage_missing_count =
                        query.counts.usage_missing_count.saturating_add(1);
                }
            },
            None => {
                query.counts.usage_missing_count =
                    query.counts.usage_missing_count.saturating_add(1);
            }
        }
        match metadata.inference_time_ms {
            Some(milliseconds) => {
                query.counts.inference_time_ms = query
                    .counts
                    .inference_time_ms
                    .saturating_add(milliseconds as u128);
                query.counts.inference_observed_count =
                    query.counts.inference_observed_count.saturating_add(1);
            }
            None => {
                query.counts.inference_missing_count =
                    query.counts.inference_missing_count.saturating_add(1);
            }
        }
    }

    pub fn record_provider_failure(&mut self, query_scope: u64, elapsed: Duration, external: bool) {
        let Some(query) = self
            .active_query
            .as_mut()
            .filter(|query| query.query_id == query_scope)
        else {
            return;
        };
        if external {
            query.counts.usage_missing_count = query.counts.usage_missing_count.saturating_add(1);
            query.counts.inference_missing_count =
                query.counts.inference_missing_count.saturating_add(1);
            if query.detailed_timing {
                query.counts.external_roundtrip_ns = query
                    .counts
                    .external_roundtrip_ns
                    .saturating_add(elapsed.as_nanos());
                query.counts.external_roundtrip_count =
                    query.counts.external_roundtrip_count.saturating_add(1);
            }
        }
    }

    pub fn record_failed_request(&mut self, query_scope: u64) {
        if let Some(query) = self
            .active_query
            .as_mut()
            .filter(|query| query.query_id == query_scope)
        {
            query.counts.failed_requests = query.counts.failed_requests.saturating_add(1);
        }
    }

    /// Opaque scopes for identities assembled during this query. Neither value contains a secret.
    pub fn identity_scopes(&self) -> (u64, u64) {
        let query_scope = self
            .active_query
            .as_ref()
            .map(|query| query.query_id)
            .unwrap_or(self.next_query_id);
        (self.connection_scope, query_scope)
    }

    pub fn credential_scope(
        &mut self,
        query_scope: u64,
        credential: Option<&str>,
    ) -> Result<u64, JudgmentError> {
        if self
            .active_query
            .as_ref()
            .is_none_or(|query| query.query_id != query_scope)
        {
            return Err(JudgmentError::provider(
                "query execution is no longer active",
            ));
        }
        let fingerprint = credential.map(|value| self.credential_hasher.hash_one(value));
        if fingerprint != self.credential_fingerprint {
            self.credential_fingerprint = fingerprint;
            self.credential_scope = NEXT_CONNECTION_SCOPE.fetch_add(1, Ordering::Relaxed);
            self.cache.clear();
        }
        Ok(self.credential_scope)
    }

    pub fn cache_mut(&mut self) -> &mut JudgmentCache {
        &mut self.cache
    }

    pub fn claim_query_entry(
        &mut self,
        query_scope: u64,
        identity: &JudgmentIdentity,
    ) -> Result<(Arc<QueryEntry>, bool), JudgmentError> {
        let active = self
            .active_query
            .as_ref()
            .filter(|query| query.query_id == query_scope)
            .ok_or_else(|| JudgmentError::provider("query execution is no longer active"))?;
        if active.cancelled.load(Ordering::Acquire) {
            return Err(JudgmentError::provider("query was cancelled"));
        }
        if let Some(entry) = self.query_entries.get(identity) {
            if let Some(query) = self
                .active_query
                .as_mut()
                .filter(|query| query.query_id == query_scope)
            {
                query.counts.deduplicated_rows = query.counts.deduplicated_rows.saturating_add(1);
            }
            return Ok((Arc::clone(entry), false));
        }
        if let Some(query) = self
            .active_query
            .as_mut()
            .filter(|query| query.query_id == query_scope)
        {
            query.counts.unique_judgments = query.counts.unique_judgments.saturating_add(1);
        }
        let entry = Arc::new(QueryEntry::new());
        self.query_entries
            .insert(identity.clone(), Arc::clone(&entry));
        Ok((entry, true))
    }

    pub fn query_cancellation(&self, query_scope: u64) -> Result<Arc<AtomicBool>, JudgmentError> {
        self.active_query
            .as_ref()
            .filter(|query| query.query_id == query_scope)
            .map(|query| Arc::clone(&query.cancelled))
            .ok_or_else(|| JudgmentError::provider("query execution is no longer active"))
    }

    pub fn peak_inflight(&self) -> usize {
        self.inflight
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .peak
    }
}

/// Acquires a connection-wide request slot without holding the runtime state mutex while waiting.
pub fn acquire_request_permit(
    connection: &SharedConnectionRuntime,
    query_scope: u64,
    max_inflight: usize,
) -> Result<InflightPermit, JudgmentError> {
    let (pool, cancelled) = {
        let runtime = connection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        (
            Arc::clone(&runtime.inflight),
            runtime.query_cancellation(query_scope)?,
        )
    };
    pool.acquire(max_inflight, &cancelled)
}

impl ExecutionRuntime for RuntimeAdapter<'_> {
    fn claim_query_entry(
        &self,
        query_scope: u64,
        identity: &JudgmentIdentity,
    ) -> Result<(Arc<QueryEntry>, bool), JudgmentError> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .claim_query_entry(query_scope, identity)
    }

    fn query_cancellation(&self, query_scope: u64) -> Result<Arc<AtomicBool>, JudgmentError> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .query_cancellation(query_scope)
    }

    fn record_cache_hit(&self, query_scope: u64) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .record_cache_hit(query_scope);
    }

    fn record_provider_attempt(&self, query_scope: u64, batch_size: usize, external: bool) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .record_provider_attempt(query_scope, batch_size, external);
    }

    fn record_failed_request(&self, query_scope: u64) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .record_failed_request(query_scope);
    }

    fn record_provider_response(
        &self,
        query_scope: u64,
        metadata: &ProviderMetadata,
        elapsed: Duration,
        external: bool,
    ) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .record_provider_response(query_scope, metadata, elapsed, external);
    }

    fn record_provider_failure(&self, query_scope: u64, elapsed: Duration, external: bool) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .record_provider_failure(query_scope, elapsed, external);
    }

    fn cache_lookup(
        &self,
        identity: Option<&JudgmentIdentity>,
        now: Instant,
        policy: &crate::cache::CachePolicy,
    ) -> Option<JudgmentResult> {
        let mut runtime = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let cache = runtime.cache_mut();
        if !cache.reconcile_policy(now, policy) {
            return None;
        }
        identity.and_then(|identity| cache.lookup(identity, now))
    }

    fn cache_insert(
        &self,
        identity: JudgmentIdentity,
        result: JudgmentResult,
        now: Instant,
        policy: &crate::cache::CachePolicy,
    ) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .cache_mut()
            .insert_success(identity, result, now, policy);
    }

    fn acquire_permit(
        &self,
        query_scope: u64,
        max_inflight: usize,
    ) -> Result<Box<dyn Send>, JudgmentError> {
        Ok(Box::new(acquire_request_permit(
            self.0,
            query_scope,
            max_inflight,
        )?))
    }
}

#[cfg(duckjeu_client_context_bridge)]
extern "C" {
    fn duckjeu_client_context_state_acquire(context: duckdb_client_context) -> *mut c_void;
}

/// Gets a cloned Rust handle to the runtime owned by DuckDB's real ClientContext.
/// The C API wrapper passed to this function remains owned by the caller.
///
/// # Safety
/// `context` must be a valid DuckDB client context for the duration of this call.
pub unsafe fn acquire_connection_runtime(
    context: duckdb_client_context,
) -> Option<SharedConnectionRuntime> {
    if context.is_null() {
        return None;
    }
    #[cfg(duckjeu_client_context_bridge)]
    {
        let handle = duckjeu_client_context_state_acquire(context);
        if handle.is_null() {
            return None;
        }
        Some(*Box::from_raw(handle.cast::<SharedConnectionRuntime>()))
    }
    #[cfg(not(duckjeu_client_context_bridge))]
    {
        let _ = context;
        None
    }
}

#[cfg(duckjeu_client_context_bridge)]
#[no_mangle]
pub extern "C" fn duckjeu_host_state_create() -> *mut c_void {
    Box::into_raw(Box::new(Arc::new(Mutex::new(ConnectionRuntime::default())))).cast()
}

#[cfg(duckjeu_client_context_bridge)]
#[no_mangle]
/// # Safety
/// `state` must be null or a live handle returned by `duckjeu_host_state_create` or
/// `duckjeu_host_state_clone`, and must not be dropped concurrently with this call.
pub unsafe extern "C" fn duckjeu_host_state_clone(state: *mut c_void) -> *mut c_void {
    if state.is_null() {
        return std::ptr::null_mut();
    }
    let runtime = &*state.cast::<SharedConnectionRuntime>();
    Box::into_raw(Box::new(Arc::clone(runtime))).cast()
}

#[cfg(duckjeu_client_context_bridge)]
#[no_mangle]
/// # Safety
/// `state` must be null or a live owned handle returned by `duckjeu_host_state_create` or
/// `duckjeu_host_state_clone`; a non-null handle must be dropped exactly once and not used
/// concurrently with this call.
pub unsafe extern "C" fn duckjeu_host_state_drop(state: *mut c_void) {
    if !state.is_null() {
        drop(Box::from_raw(state.cast::<SharedConnectionRuntime>()));
    }
}

#[cfg(duckjeu_client_context_bridge)]
fn lock_runtime(runtime: &SharedConnectionRuntime) -> MutexGuard<'_, ConnectionRuntime> {
    runtime
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(duckjeu_client_context_bridge)]
unsafe fn with_runtime(state: *mut c_void, callback: impl FnOnce(&mut ConnectionRuntime)) {
    if state.is_null() {
        return;
    }
    let runtime = &*state.cast::<SharedConnectionRuntime>();
    let _ = catch_unwind(AssertUnwindSafe(|| callback(&mut lock_runtime(runtime))));
}

#[cfg(duckjeu_client_context_bridge)]
#[no_mangle]
/// # Safety
/// `state` must be null or a live handle returned by `duckjeu_host_state_create` or
/// `duckjeu_host_state_clone`, and must not be dropped concurrently with this call.
pub unsafe extern "C" fn duckjeu_host_query_begin(state: *mut c_void) {
    with_runtime(state, ConnectionRuntime::begin_query);
}

#[cfg(duckjeu_client_context_bridge)]
#[no_mangle]
/// # Safety
/// `state` must be null or a live handle returned by `duckjeu_host_state_create` or
/// `duckjeu_host_state_clone`, and must not be dropped concurrently with this call.
pub unsafe extern "C" fn duckjeu_host_query_end(state: *mut c_void, error_type: u32) {
    with_runtime(state, |runtime| runtime.end_query(error_type));
}

#[cfg(duckjeu_client_context_bridge)]
#[no_mangle]
/// # Safety
/// `error_data` must be null or a valid DuckDB error-data pointer for the duration of this call.
pub unsafe extern "C" fn duckjeu_host_error_type(error_data: *mut c_void) -> u32 {
    if error_data.is_null() {
        return duckdb_error_type_DUCKDB_ERROR_INVALID_TYPE;
    }
    duckdb_error_data_error_type(error_data.cast())
}

#[cfg(test)]
mod tests {
    use super::{ConnectionRuntime, QueryOutcome};
    use crate::judgment::{
        JudgmentIdentity, JudgmentIdentityContext, JudgmentRequest, JudgmentResult,
    };
    use crate::serialize::encode_text;
    use duckdb::ffi::{
        duckdb_error_type_DUCKDB_ERROR_INTERRUPT, duckdb_error_type_DUCKDB_ERROR_INVALID_INPUT,
    };

    #[test]
    fn query_lifecycle_records_success_failure_and_cancellation() {
        let mut runtime = ConnectionRuntime::default();

        runtime.begin_query();
        let scope = runtime.identity_scopes().1;
        runtime.record_row_input(scope);
        runtime.end_query(0);
        let success = runtime.last_query().unwrap();
        assert_eq!(success.query_id, 0);
        assert_eq!(success.outcome, QueryOutcome::Succeeded);

        runtime.begin_query();
        let scope = runtime.identity_scopes().1;
        runtime.record_row_input(scope);
        runtime.end_query(duckdb_error_type_DUCKDB_ERROR_INVALID_INPUT);
        let failure = runtime.last_query().unwrap();
        assert_eq!(failure.query_id, 1);
        assert_eq!(failure.outcome, QueryOutcome::Failed);

        runtime.begin_query();
        let scope = runtime.identity_scopes().1;
        runtime.record_row_input(scope);
        runtime.end_query(duckdb_error_type_DUCKDB_ERROR_INTERRUPT);
        let cancellation = runtime.last_query().unwrap();
        assert_eq!(cancellation.query_id, 2);
        assert_eq!(cancellation.outcome, QueryOutcome::Cancelled);
    }

    #[test]
    fn query_counts_separate_dedup_cache_provider_and_network_attempts() {
        let mut runtime = ConnectionRuntime::default();
        runtime.begin_query();
        let query_scope = runtime.identity_scopes().1;
        runtime.record_optimized_rows(query_scope, 4, 1, 3, 2);
        for state in ["first", "second"] {
            let request = JudgmentRequest::noul(encode_text(state), "criterion").unwrap();
            let identity = JudgmentIdentity::new(
                &request,
                JudgmentIdentityContext {
                    provider_namespace: "typesafe".into(),
                    endpoint: "https://api.typesafe.ai/v1/systemone".into(),
                    requested_model: "jev-latest".into(),
                    effective_model: None,
                    credential_scope: 1,
                    model_binding_scope: query_scope,
                },
            );
            runtime.claim_query_entry(query_scope, &identity).unwrap();
        }
        runtime.record_cache_hit(query_scope);
        runtime.record_provider_attempt(query_scope, 2, true);
        runtime.record_provider_attempt(query_scope, 1, false);
        runtime.record_failed_request(query_scope);
        runtime.end_query(duckdb_error_type_DUCKDB_ERROR_INVALID_INPUT);

        let counts = &runtime.last_query().unwrap().counts;
        assert_eq!(counts.input_rows, 4);
        assert_eq!(counts.null_rows, 1);
        assert_eq!(counts.valid_rows, 3);
        assert_eq!(counts.unique_judgments, 2);
        assert_eq!(counts.deduplicated_rows, 1);
        assert_eq!(counts.cache_hit_judgments, 1);
        assert_eq!(counts.provider_invocations, 2);
        assert_eq!(counts.external_requests, 1);
        assert_eq!(counts.batched_requests, 1);
        assert_eq!(counts.batch_size_histogram.get(&2), Some(&1));
        assert_eq!(counts.failed_requests, 1);
    }

    #[test]
    fn row_mode_counts_nulls_and_provider_requests_without_deduplication() {
        let mut runtime = ConnectionRuntime::default();
        runtime.begin_query();
        let query_scope = runtime.identity_scopes().1;
        runtime.record_row_input(query_scope);
        runtime.record_row_result(query_scope, false);
        runtime.record_provider_attempt(query_scope, 1, true);
        runtime.record_row_input(query_scope);
        runtime.record_row_result(query_scope, true);
        runtime.end_query(0);

        let counts = &runtime.last_query().unwrap().counts;
        assert_eq!(counts.input_rows, 2);
        assert_eq!(counts.valid_rows, 1);
        assert_eq!(counts.unique_judgments, 1);
        assert_eq!(counts.null_rows, 1);
        assert_eq!(counts.deduplicated_rows, 0);
        assert_eq!(counts.external_requests, 1);
    }

    #[test]
    fn temporary_dedup_results_are_cleared_when_query_ends() {
        let mut runtime = ConnectionRuntime::default();
        runtime.begin_query();
        let query_scope = runtime.identity_scopes().1;
        let request = JudgmentRequest::noul(encode_text("private state"), "q").unwrap();
        let identity = JudgmentIdentity::new(
            &request,
            JudgmentIdentityContext {
                provider_namespace: "mock".into(),
                endpoint: "local".into(),
                requested_model: "mock-v1".into(),
                effective_model: None,
                credential_scope: runtime.identity_scopes().0,
                model_binding_scope: runtime.identity_scopes().1,
            },
        );
        let (entry, owner) = runtime.claim_query_entry(query_scope, &identity).unwrap();
        assert!(owner);
        entry.complete(Ok(JudgmentResult::Noul(0.5)));
        assert_eq!(
            entry
                .wait(&runtime.query_cancellation(query_scope).unwrap())
                .unwrap(),
            JudgmentResult::Noul(0.5)
        );

        runtime.end_query(0);
        assert!(runtime.claim_query_entry(query_scope, &identity).is_err());
    }

    #[test]
    fn credential_scope_rotates_and_clears_cache_without_retaining_secret() {
        use std::time::{Duration, Instant};

        use crate::cache::CachePolicy;

        let mut runtime = ConnectionRuntime::default();
        runtime.begin_query();
        let query_scope = runtime.identity_scopes().1;
        let initial_scope = runtime.credential_scope(query_scope, None).unwrap();
        assert_eq!(
            runtime.credential_scope(query_scope, None).unwrap(),
            initial_scope
        );

        let request = JudgmentRequest::noul(encode_text("state"), "q").unwrap();
        let identity = JudgmentIdentity::new(
            &request,
            JudgmentIdentityContext {
                provider_namespace: "mock".into(),
                endpoint: "local".into(),
                requested_model: "mock".into(),
                effective_model: Some("mock-v1".into()),
                credential_scope: initial_scope,
                model_binding_scope: query_scope,
            },
        );
        let policy = CachePolicy {
            max_entries: 2,
            max_bytes: 10_000,
            ttl: Duration::from_secs(10),
        };
        runtime.cache_mut().insert_success(
            identity,
            JudgmentResult::Noul(0.5),
            Instant::now(),
            &policy,
        );
        assert_eq!(runtime.cache_mut().len(Instant::now()), 1);

        let token_scope = runtime
            .credential_scope(query_scope, Some("api-key-a"))
            .unwrap();
        assert_ne!(token_scope, initial_scope);
        assert_eq!(runtime.cache_mut().len(Instant::now()), 0);
        assert_eq!(
            runtime
                .credential_scope(query_scope, Some("api-key-a"))
                .unwrap(),
            token_scope
        );
        let rotated_scope = runtime
            .credential_scope(query_scope, Some("api-key-b"))
            .unwrap();
        assert_ne!(rotated_scope, token_scope);
    }

    #[test]
    fn request_permits_enforce_connection_limit_and_cancel_waiters() {
        use std::sync::{mpsc, Arc, Mutex};
        use std::time::Duration;

        use super::acquire_request_permit;

        let shared = Arc::new(Mutex::new(ConnectionRuntime::default()));
        let query_scope = {
            let mut runtime = shared.lock().unwrap();
            runtime.begin_query();
            runtime.identity_scopes().1
        };
        let first = acquire_request_permit(&shared, query_scope, 1).unwrap();
        let thread_runtime = Arc::clone(&shared);
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let permit = acquire_request_permit(&thread_runtime, query_scope, 1);
            acquired_tx.send(permit.is_ok()).unwrap();
            drop(permit);
        });
        assert!(acquired_rx.recv_timeout(Duration::from_millis(40)).is_err());
        drop(first);
        assert!(acquired_rx.recv_timeout(Duration::from_secs(1)).unwrap());
        waiter.join().unwrap();
        assert_eq!(shared.lock().unwrap().peak_inflight(), 1);

        let held = acquire_request_permit(&shared, query_scope, 1).unwrap();
        let thread_runtime = Arc::clone(&shared);
        let (started_tx, started_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            result_tx
                .send(acquire_request_permit(&thread_runtime, query_scope, 1).is_err())
                .unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        shared
            .lock()
            .unwrap()
            .end_query(duckdb_error_type_DUCKDB_ERROR_INTERRUPT);
        assert!(result_rx.recv_timeout(Duration::from_secs(1)).unwrap());
        waiter.join().unwrap();
        drop(held);
    }
}
