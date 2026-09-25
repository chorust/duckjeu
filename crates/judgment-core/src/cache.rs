//! Bounded in-memory cache for fully validated judgment successes.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use crate::judgment::{JudgmentError, JudgmentIdentity, JudgmentResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachePolicy {
    pub max_entries: usize,
    pub max_bytes: usize,
    pub ttl: Duration,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheCounters {
    pub expirations: u64,
    pub evictions: u64,
}

impl CachePolicy {
    pub fn validate(&self) -> Result<(), JudgmentError> {
        if self.max_entries == 0 || self.max_bytes == 0 || self.ttl.is_zero() {
            return Err(JudgmentError::configuration(
                "cache entry, byte, and TTL limits must be positive",
            ));
        }
        Ok(())
    }
}

struct CacheEntry {
    result: JudgmentResult,
    inserted_at: Instant,
    expires_at: Instant,
    estimated_bytes: usize,
}

/// Per-connection LRU cache. It has no disk representation and accepts successes only.
#[derive(Default)]
pub struct JudgmentCache {
    entries: HashMap<JudgmentIdentity, CacheEntry>,
    lru: VecDeque<JudgmentIdentity>,
    estimated_bytes: usize,
    counters: CacheCounters,
}

impl JudgmentCache {
    pub fn lookup(&mut self, identity: &JudgmentIdentity, now: Instant) -> Option<JudgmentResult> {
        let expired = self
            .entries
            .get(identity)
            .is_some_and(|entry| now >= entry.expires_at);
        if expired {
            self.remove(identity);
            self.counters.expirations = self.counters.expirations.saturating_add(1);
            return None;
        }
        let result = self.entries.get(identity)?.result.clone();
        self.touch(identity);
        Some(result)
    }

    /// Applies the active limits before allowing an entry to be served.
    pub fn lookup_with_policy(
        &mut self,
        identity: &JudgmentIdentity,
        now: Instant,
        policy: &CachePolicy,
    ) -> Option<JudgmentResult> {
        if !self.reconcile_policy(now, policy) {
            return None;
        }
        self.lookup(identity, now)
    }

    /// Expires entries under the current TTL and evicts least-recently-used entries until the
    /// current entry and byte limits are satisfied. Returns false for an invalid policy.
    pub fn reconcile_policy(&mut self, now: Instant, policy: &CachePolicy) -> bool {
        if policy.validate().is_err() {
            return false;
        }
        for entry in self.entries.values_mut() {
            if let Some(policy_expiry) = entry.inserted_at.checked_add(policy.ttl) {
                entry.expires_at = entry.expires_at.min(policy_expiry);
            }
        }
        self.prune_expired(now);
        while self.entries.len() > policy.max_entries || self.estimated_bytes > policy.max_bytes {
            let Some(oldest) = self.lru.pop_front() else {
                break;
            };
            self.remove_entry(&oldest);
            self.counters.evictions = self.counters.evictions.saturating_add(1);
        }
        true
    }

    pub fn insert_outcome(
        &mut self,
        identity: JudgmentIdentity,
        outcome: &Result<JudgmentResult, JudgmentError>,
        now: Instant,
        policy: &CachePolicy,
    ) -> bool {
        let Ok(result) = outcome else {
            return false;
        };
        self.insert_success(identity, result.clone(), now, policy)
    }

    pub fn insert_success(
        &mut self,
        identity: JudgmentIdentity,
        result: JudgmentResult,
        now: Instant,
        policy: &CachePolicy,
    ) -> bool {
        if policy.validate().is_err() {
            return false;
        }
        let Some(expires_at) = now.checked_add(policy.ttl) else {
            return false;
        };
        self.reconcile_policy(now, policy);
        let estimated_bytes = Self::estimated_entry_bytes(&identity, &result);
        if estimated_bytes > policy.max_bytes {
            self.remove(&identity);
            return false;
        }

        self.remove(&identity);
        while self.entries.len() >= policy.max_entries
            || self.estimated_bytes.saturating_add(estimated_bytes) > policy.max_bytes
        {
            let Some(oldest) = self.lru.pop_front() else {
                return false;
            };
            self.remove_entry(&oldest);
            self.counters.evictions = self.counters.evictions.saturating_add(1);
        }

        self.estimated_bytes = self.estimated_bytes.saturating_add(estimated_bytes);
        self.lru.push_back(identity.clone());
        self.entries.insert(
            identity,
            CacheEntry {
                result,
                inserted_at: now,
                expires_at,
                estimated_bytes,
            },
        );
        true
    }

    pub fn clear(&mut self) -> usize {
        let removed = self.entries.len();
        self.entries.clear();
        self.lru.clear();
        self.estimated_bytes = 0;
        removed
    }

    pub fn len(&mut self, now: Instant) -> usize {
        self.prune_expired(now);
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn estimated_bytes(&self) -> usize {
        self.estimated_bytes
    }

    pub fn counters(&self) -> CacheCounters {
        self.counters
    }

    /// Conservative accounting for key data, result payload, and map/LRU bookkeeping.
    pub fn estimated_entry_bytes(identity: &JudgmentIdentity, result: &JudgmentResult) -> usize {
        let text_bytes = identity
            .criterion
            .as_ref()
            .map_or(0, String::len)
            .saturating_add(identity.question.as_ref().map_or(0, String::len))
            .saturating_add(
                identity
                    .choices
                    .iter()
                    .fold(0usize, |sum, value| sum.saturating_add(value.len())),
            )
            .saturating_add(identity.provider_namespace.len())
            .saturating_add(identity.endpoint.len())
            .saturating_add(identity.requested_model.len())
            .saturating_add(identity.effective_model.as_ref().map_or(0, String::len));
        let result_bytes = match result {
            JudgmentResult::Noul(_) => std::mem::size_of::<f64>(),
            JudgmentResult::Choice(label) => label.len(),
        };
        512usize
            .saturating_add(identity.canonical_bytes.len().saturating_mul(6))
            .saturating_add(text_bytes.saturating_mul(6))
            .saturating_add(result_bytes)
    }

    fn touch(&mut self, identity: &JudgmentIdentity) {
        if let Some(index) = self.lru.iter().position(|item| item == identity) {
            self.lru.remove(index);
        }
        self.lru.push_back(identity.clone());
    }

    fn remove(&mut self, identity: &JudgmentIdentity) {
        self.remove_entry(identity);
        if let Some(index) = self.lru.iter().position(|item| item == identity) {
            self.lru.remove(index);
        }
    }

    fn remove_entry(&mut self, identity: &JudgmentIdentity) {
        if let Some(entry) = self.entries.remove(identity) {
            self.estimated_bytes = self.estimated_bytes.saturating_sub(entry.estimated_bytes);
        }
    }

    fn prune_expired(&mut self, now: Instant) {
        let expired: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| now >= entry.expires_at)
            .map(|(identity, _)| identity.clone())
            .collect();
        for identity in expired {
            self.remove(&identity);
            self.counters.expirations = self.counters.expirations.saturating_add(1);
        }
    }
}
