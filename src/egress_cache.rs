//! Small bounded TTL caches for the egress hot path.
//!
//! `POST /egress/{contract_id}` repeats four DB lookups on every call — the
//! contract identity, the pinned version row, the latest-stable version (for the
//! default pin), and the org payload-storage plan check.  For a busy contract
//! these answers barely change, yet each request paid for all four round-trips.
//!
//! [`EgressCache`] memoises them behind a short TTL (default 45 s) with a bounded
//! entry count, so steady-state egress traffic resolves from memory.  There is
//! no `moka` dependency in this crate, so this is a deliberately small in-house
//! cache rather than a full-featured one.
//!
//! ## Staleness
//!
//! Entries live for `TTL_SECONDS`.  The contract/version caches are invalidated
//! eagerly from `AppState::invalidate_version` / `invalidate_contract_all` (the
//! draft-edit and delete paths), so edits are visible immediately.  The
//! payload-policy cache is keyed by org, not contract, and is **not** eagerly
//! invalidated: a plan change can take up to `TTL_SECONDS` to take effect on the
//! egress path.  That window is intentional and bounded.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::contract::{ContractIdentity, ContractVersion};

/// TTL applied to every egress cache entry.  Kept in the 30–60 s band so a
/// stale plan/version answer self-heals quickly.
pub const TTL_SECONDS: u64 = 45;

/// Per-cache entry cap.  Egress traffic concentrates on a handful of contracts,
/// so this is comfortably above the working set while bounding memory.
const MAX_ENTRIES: usize = 4096;

/// A minimal bounded TTL cache.
///
/// Insert-time housekeeping drops expired entries first, then evicts the oldest
/// surviving entry if the cache is still at capacity — no background task.
struct TtlCache<K, V> {
    ttl: Duration,
    max_entries: usize,
    map: Mutex<HashMap<K, (Instant, V)>>,
}

impl<K: Eq + Hash + Clone, V: Clone> TtlCache<K, V> {
    fn new(ttl: Duration, max_entries: usize) -> Self {
        TtlCache {
            ttl,
            max_entries,
            map: Mutex::new(HashMap::new()),
        }
    }

    fn get_at(&self, key: &K, now: Instant) -> Option<V> {
        let map = self.map.lock().unwrap();
        match map.get(key) {
            Some((inserted, value)) if now.duration_since(*inserted) < self.ttl => {
                Some(value.clone())
            }
            _ => None,
        }
    }

    fn insert_at(&self, key: K, value: V, now: Instant) {
        let mut map = self.map.lock().unwrap();
        map.retain(|_, (inserted, _)| now.duration_since(*inserted) < self.ttl);
        if map.len() >= self.max_entries && !map.contains_key(&key) {
            // Evict the oldest surviving entry to stay bounded.
            if let Some(oldest) = map
                .iter()
                .min_by_key(|(_, (inserted, _))| *inserted)
                .map(|(k, _)| k.clone())
            {
                map.remove(&oldest);
            }
        }
        map.insert(key, (now, value));
    }

    fn get(&self, key: &K) -> Option<V> {
        self.get_at(key, Instant::now())
    }

    fn insert(&self, key: K, value: V) {
        self.insert_at(key, value, Instant::now())
    }

    /// Drop every entry whose key satisfies `pred`.
    fn invalidate_where(&self, pred: impl Fn(&K) -> bool) {
        let mut map = self.map.lock().unwrap();
        map.retain(|k, _| !pred(k));
    }
}

/// Caches for the four egress hot-path lookups.
pub struct EgressCache {
    /// (contract_id, org_id) → identity.
    identity: TtlCache<(Uuid, Option<Uuid>), ContractIdentity>,
    /// (contract_id, pin, org_id) → resolved version row.
    version: TtlCache<(Uuid, String, Option<Uuid>), ContractVersion>,
    /// contract_id → latest stable version (None = no stable version).
    latest_stable: TtlCache<Uuid, Option<ContractVersion>>,
    /// (org_id, contract_switch) → payload-storage decision.
    payload_policy: TtlCache<(Option<Uuid>, bool), bool>,
}

impl Default for EgressCache {
    fn default() -> Self {
        let ttl = Duration::from_secs(TTL_SECONDS);
        EgressCache {
            identity: TtlCache::new(ttl, MAX_ENTRIES),
            version: TtlCache::new(ttl, MAX_ENTRIES),
            latest_stable: TtlCache::new(ttl, MAX_ENTRIES),
            payload_policy: TtlCache::new(ttl, MAX_ENTRIES),
        }
    }
}

impl EgressCache {
    pub fn identity_get(
        &self,
        contract_id: Uuid,
        org_id: Option<Uuid>,
    ) -> Option<ContractIdentity> {
        self.identity.get(&(contract_id, org_id))
    }
    pub fn identity_put(&self, contract_id: Uuid, org_id: Option<Uuid>, v: ContractIdentity) {
        self.identity.insert((contract_id, org_id), v);
    }

    pub fn version_get(
        &self,
        contract_id: Uuid,
        pin: &str,
        org_id: Option<Uuid>,
    ) -> Option<ContractVersion> {
        self.version.get(&(contract_id, pin.to_string(), org_id))
    }
    pub fn version_put(
        &self,
        contract_id: Uuid,
        pin: &str,
        org_id: Option<Uuid>,
        v: ContractVersion,
    ) {
        self.version
            .insert((contract_id, pin.to_string(), org_id), v);
    }

    pub fn latest_stable_get(&self, contract_id: Uuid) -> Option<Option<ContractVersion>> {
        self.latest_stable.get(&contract_id)
    }
    pub fn latest_stable_put(&self, contract_id: Uuid, v: Option<ContractVersion>) {
        self.latest_stable.insert(contract_id, v);
    }

    pub fn payload_policy_get(&self, org_id: Option<Uuid>, contract_switch: bool) -> Option<bool> {
        self.payload_policy.get(&(org_id, contract_switch))
    }
    pub fn payload_policy_put(&self, org_id: Option<Uuid>, contract_switch: bool, v: bool) {
        self.payload_policy.insert((org_id, contract_switch), v);
    }

    /// Drop cached contract/version answers for a single version. Called from
    /// the draft-edit path so an edit is visible to egress immediately.
    pub fn invalidate_version(&self, contract_id: Uuid, version: &str) {
        self.identity
            .invalidate_where(|(cid, _)| *cid == contract_id);
        self.version
            .invalidate_where(|(cid, v, _)| *cid == contract_id && v == version);
        self.latest_stable
            .invalidate_where(|cid| *cid == contract_id);
    }

    /// Drop every cached answer for a contract. Called on delete.
    pub fn invalidate_contract_all(&self, contract_id: Uuid) {
        self.identity
            .invalidate_where(|(cid, _)| *cid == contract_id);
        self.version
            .invalidate_where(|(cid, _, _)| *cid == contract_id);
        self.latest_stable
            .invalidate_where(|cid| *cid == contract_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_within_ttl_miss_after_expiry() {
        let cache: TtlCache<u32, &str> = TtlCache::new(Duration::from_secs(45), 16);
        let t0 = Instant::now();
        cache.insert_at(1, "a", t0);

        assert_eq!(cache.get_at(&1, t0 + Duration::from_secs(10)), Some("a"));
        assert_eq!(cache.get_at(&1, t0 + Duration::from_secs(44)), Some("a"));
        // At/after the TTL the entry is gone.
        assert_eq!(cache.get_at(&1, t0 + Duration::from_secs(45)), None);
        assert_eq!(cache.get_at(&1, t0 + Duration::from_secs(60)), None);
    }

    #[test]
    fn eviction_keeps_cache_bounded() {
        let cache: TtlCache<u32, u32> = TtlCache::new(Duration::from_secs(45), 3);
        let t0 = Instant::now();
        // Insert 1,2,3 at increasing times, then a 4th — oldest (key 1) is evicted.
        cache.insert_at(1, 1, t0);
        cache.insert_at(2, 2, t0 + Duration::from_secs(1));
        cache.insert_at(3, 3, t0 + Duration::from_secs(2));
        cache.insert_at(4, 4, t0 + Duration::from_secs(3));

        let now = t0 + Duration::from_secs(4);
        assert_eq!(cache.get_at(&1, now), None, "oldest entry evicted");
        assert_eq!(cache.get_at(&2, now), Some(2));
        assert_eq!(cache.get_at(&3, now), Some(3));
        assert_eq!(cache.get_at(&4, now), Some(4));
        assert_eq!(cache.map.lock().unwrap().len(), 3);
    }

    #[test]
    fn expired_entries_cleared_on_insert() {
        let cache: TtlCache<u32, u32> = TtlCache::new(Duration::from_secs(45), 100);
        let t0 = Instant::now();
        cache.insert_at(1, 1, t0);
        cache.insert_at(2, 2, t0);
        // Insert well past the TTL — the stale entries are swept first.
        cache.insert_at(3, 3, t0 + Duration::from_secs(50));
        assert_eq!(cache.map.lock().unwrap().len(), 1);
        assert_eq!(cache.get_at(&3, t0 + Duration::from_secs(50)), Some(3));
    }

    #[test]
    fn invalidate_where_drops_matching_keys() {
        let cache: TtlCache<(u32, u32), u32> = TtlCache::new(Duration::from_secs(45), 16);
        let t0 = Instant::now();
        cache.insert_at((1, 10), 1, t0);
        cache.insert_at((1, 11), 2, t0);
        cache.insert_at((2, 10), 3, t0);
        cache.invalidate_where(|(a, _)| *a == 1);
        assert_eq!(cache.get_at(&(1, 10), t0), None);
        assert_eq!(cache.get_at(&(1, 11), t0), None);
        assert_eq!(cache.get_at(&(2, 10), t0), Some(3));
    }
}
