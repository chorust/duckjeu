//! Host-injected execution state used by the shared executor.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::cache::CachePolicy;
use crate::judgment::{JudgmentError, JudgmentIdentity, JudgmentResult};
use crate::provider::ProviderMetadata;

pub struct QueryEntry {
    result: Mutex<Option<Result<JudgmentResult, JudgmentError>>>,
    changed: Condvar,
}

impl QueryEntry {
    pub fn new() -> Self {
        Self {
            result: Mutex::new(None),
            changed: Condvar::new(),
        }
    }
    pub fn complete(&self, result: Result<JudgmentResult, JudgmentError>) {
        let mut slot = self.result.lock().unwrap_or_else(|p| p.into_inner());
        if slot.is_none() {
            *slot = Some(result);
        }
        self.changed.notify_all();
    }
    pub fn wait(&self, cancelled: &AtomicBool) -> Result<JudgmentResult, JudgmentError> {
        let mut slot = self.result.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            if let Some(result) = slot.as_ref() {
                return result.clone();
            }
            if cancelled.load(std::sync::atomic::Ordering::Acquire) {
                return Err(JudgmentError::provider(
                    "query was cancelled while awaiting a judgment",
                ));
            }
            let (next, _) = self
                .changed
                .wait_timeout(slot, Duration::from_millis(20))
                .unwrap_or_else(|p| p.into_inner());
            slot = next;
        }
    }
}

impl Default for QueryEntry {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ExecutionRuntime: Send + Sync {
    fn claim_query_entry(
        &self,
        query_scope: u64,
        identity: &JudgmentIdentity,
    ) -> Result<(Arc<QueryEntry>, bool), JudgmentError>;
    fn query_cancellation(&self, query_scope: u64) -> Result<Arc<AtomicBool>, JudgmentError>;
    fn record_cache_hit(&self, query_scope: u64);
    fn record_provider_attempt(&self, query_scope: u64, batch_size: usize, external: bool);
    fn record_failed_request(&self, query_scope: u64);
    fn record_provider_response(
        &self,
        query_scope: u64,
        metadata: &ProviderMetadata,
        elapsed: Duration,
        external: bool,
    );
    fn record_provider_failure(&self, query_scope: u64, elapsed: Duration, external: bool);
    fn cache_lookup(
        &self,
        identity: Option<&JudgmentIdentity>,
        now: Instant,
        policy: &CachePolicy,
    ) -> Option<JudgmentResult>;
    fn cache_insert(
        &self,
        identity: JudgmentIdentity,
        result: JudgmentResult,
        now: Instant,
        policy: &CachePolicy,
    );
    fn acquire_permit(
        &self,
        query_scope: u64,
        max_inflight: usize,
    ) -> Result<Box<dyn Send>, JudgmentError>;
}
