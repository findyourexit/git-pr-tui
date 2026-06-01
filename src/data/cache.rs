//! In-memory cache layer for fetched GitHub resources.
//!
//! Every cached value lives inside a `CacheEntry { fetched_at, version, value }`.
//! `version` is incremented on every put for a given key, allowing the reducer to
//! detect stale writes: capture the version at submit time, compare against the
//! current version when the response lands.

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::sync::RwLock;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct CacheEntry<V> {
    pub fetched_at: Instant,
    pub version: u64,
    pub value: V,
}

#[derive(Debug)]
pub struct VersionedCache<K, V> {
    ttl: Duration,
    inner: RwLock<HashMap<K, CacheEntry<V>>>,
}

impl<K: Eq + Hash + Clone, V: Clone> VersionedCache<K, V> {
    #[must_use]
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            inner: RwLock::new(HashMap::new()),
        }
    }

    #[must_use]
    pub fn get(&self, key: &K) -> Option<CacheEntry<V>> {
        let guard = self.inner.read().ok()?;
        let entry = guard.get(key)?;
        if entry.fetched_at.elapsed() > self.ttl {
            None
        } else {
            Some(entry.clone())
        }
    }

    /// Returns the new version for `key`. Versions are monotonic per key and
    /// survive `invalidate` so stale-write detection works across refreshes.
    pub fn put(&self, key: K, value: V) -> u64 {
        let mut guard = self.inner.write().expect("cache lock poisoned");
        let v = guard.get(&key).map_or(1, |e| e.version + 1);
        guard.insert(
            key,
            CacheEntry {
                fetched_at: Instant::now(),
                version: v,
                value,
            },
        );
        v
    }

    pub fn invalidate(&self, key: &K) {
        if let Ok(mut guard) = self.inner.write() {
            // Tombstone via backdated `fetched_at` — preserves version counter
            // so stale writes submitted before invalidate remain detectable.
            if let Some(entry) = guard.get_mut(key) {
                entry.fetched_at = Self::expired_instant(self.ttl);
            }
        }
    }

    /// Tombstone the entry AND bump its version counter so any in-flight stale
    /// write keyed off the previous version is rejected on completion.
    /// Returns the new version, or `0` when `key` is not yet cached (callers
    /// must `put` the fresh value before relying on the bumped version).
    pub fn invalidate_and_bump(&self, key: &K) -> u64 {
        let Ok(mut guard) = self.inner.write() else {
            return 0;
        };
        let Some(entry) = guard.get_mut(key) else {
            return 0;
        };
        entry.version += 1;
        entry.fetched_at = Self::expired_instant(self.ttl);
        entry.version
    }

    pub fn invalidate_matching<F: Fn(&K) -> bool>(&self, pred: F) {
        if let Ok(mut guard) = self.inner.write() {
            let stamp = Self::expired_instant(self.ttl);
            for (k, entry) in guard.iter_mut() {
                if pred(k) {
                    entry.fetched_at = stamp;
                }
            }
        }
    }

    #[must_use]
    pub fn current_version(&self, key: &K) -> u64 {
        self.inner
            .read()
            .ok()
            .and_then(|g| g.get(key).map(|e| e.version))
            .unwrap_or(0)
    }

    /// Returns an `Instant` known to be expired relative to `ttl`. Saturates
    /// at `now` if subtraction would underflow (e.g. very small TTLs near
    /// process start) — callers must accept that on such platforms a
    /// freshly tombstoned entry may briefly read as still-fresh.
    fn expired_instant(ttl: Duration) -> Instant {
        let now = Instant::now();
        now.checked_sub(ttl)
            .and_then(|i| i.checked_sub(Duration::from_secs(1)))
            .unwrap_or(now)
    }
}

#[cfg(test)]
mod versioned_tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn put_returns_monotonic_versions_for_same_key() {
        let c: VersionedCache<&'static str, u32> = VersionedCache::new(Duration::from_secs(60));
        assert_eq!(c.put("k", 1), 1);
        assert_eq!(c.put("k", 2), 2);
        assert_eq!(c.put("k", 3), 3);
    }

    #[test]
    fn current_version_zero_for_unknown_key() {
        let c: VersionedCache<&'static str, u32> = VersionedCache::new(Duration::from_secs(60));
        assert_eq!(c.current_version(&"missing"), 0);
    }

    #[test]
    fn get_returns_value_within_ttl() {
        let c: VersionedCache<&'static str, u32> = VersionedCache::new(Duration::from_secs(60));
        c.put("k", 42);
        let e = c.get(&"k").unwrap();
        assert_eq!(e.value, 42);
        assert_eq!(e.version, 1);
    }

    #[test]
    fn expires_after_ttl() {
        let c: VersionedCache<&'static str, u32> = VersionedCache::new(Duration::from_millis(20));
        c.put("k", 1);
        sleep(Duration::from_millis(40));
        assert!(c.get(&"k").is_none());
    }

    #[test]
    fn invalidate_matching_drops_only_matching_keys() {
        let c: VersionedCache<String, u32> = VersionedCache::new(Duration::from_secs(60));
        c.put("dashboard:review-requested".into(), 1);
        c.put("dashboard:authored".into(), 2);
        c.put("pr_list:acme/widgets".into(), 3);
        c.invalidate_matching(|k| k.starts_with("dashboard:"));
        assert!(c.get(&"dashboard:review-requested".to_string()).is_none());
        assert!(c.get(&"dashboard:authored".to_string()).is_none());
        assert!(c.get(&"pr_list:acme/widgets".to_string()).is_some());
    }

    #[test]
    fn invalidate_does_not_reset_version() {
        // Version is a monotonic counter per key — must survive invalidation
        // so a stale write submitted before invalidate is still detectable.
        let c: VersionedCache<&'static str, u32> = VersionedCache::new(Duration::from_secs(60));
        c.put("k", 1);
        c.put("k", 2);
        c.invalidate(&"k");
        assert_eq!(
            c.put("k", 3),
            3,
            "version must continue from before invalidate"
        );
    }

    #[test]
    fn invalidate_and_bump_increments_version_and_tombstones() {
        let c: VersionedCache<&'static str, u32> = VersionedCache::new(Duration::from_secs(60));
        c.put("k", 1);
        c.put("k", 2);
        assert_eq!(c.invalidate_and_bump(&"k"), 3);
        assert!(c.get(&"k").is_none(), "must be tombstoned after bump");
        assert_eq!(
            c.put("k", 4),
            4,
            "next put continues monotonic sequence past the bump"
        );
    }

    #[test]
    fn invalidate_and_bump_returns_zero_for_unknown_key() {
        let c: VersionedCache<&'static str, u32> = VersionedCache::new(Duration::from_secs(60));
        assert_eq!(c.invalidate_and_bump(&"missing"), 0);
    }
}

/// LRU cache for raw diff strings; capacity fixed to 16 entries.
#[derive(Debug)]
pub struct LruDiffCache<K, V> {
    capacity: usize,
    order: RwLock<VecDeque<K>>,
    inner: RwLock<HashMap<K, V>>,
}

pub const DIFF_CACHE_CAPACITY: usize = 16;

impl<K: Eq + Hash + Clone, V: Clone> LruDiffCache<K, V> {
    #[must_use]
    pub fn with_default_capacity() -> Self {
        Self::new(DIFF_CACHE_CAPACITY)
    }

    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            order: RwLock::new(VecDeque::with_capacity(capacity)),
            inner: RwLock::new(HashMap::with_capacity(capacity)),
        }
    }

    #[must_use]
    pub fn get(&self, key: &K) -> Option<V> {
        let value = self.inner.read().ok()?.get(key).cloned()?;
        if let Ok(mut order) = self.order.write() {
            order.retain(|k| k != key);
            order.push_back(key.clone());
        }
        Some(value)
    }

    pub fn put(&self, key: K, value: V) {
        let Ok(mut inner) = self.inner.write() else {
            return;
        };
        let Ok(mut order) = self.order.write() else {
            return;
        };
        if inner.contains_key(&key) {
            order.retain(|k| k != &key);
        } else if inner.len() >= self.capacity
            && let Some(victim) = order.pop_front()
        {
            inner.remove(&victim);
        }
        order.push_back(key.clone());
        inner.insert(key, value);
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.read().map_or(0, |g| g.len())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod lru_tests {
    use super::*;

    #[test]
    fn default_capacity_is_sixteen() {
        assert_eq!(DIFF_CACHE_CAPACITY, 16);
    }

    #[test]
    fn evicts_oldest_when_full() {
        let cache: LruDiffCache<u32, &'static str> = LruDiffCache::new(2);
        cache.put(1, "a");
        cache.put(2, "b");
        cache.put(3, "c");
        assert_eq!(cache.get(&1), None);
        assert_eq!(cache.get(&2), Some("b"));
        assert_eq!(cache.get(&3), Some("c"));
    }

    #[test]
    fn get_promotes_to_most_recent() {
        let cache: LruDiffCache<u32, &'static str> = LruDiffCache::new(2);
        cache.put(1, "a");
        cache.put(2, "b");
        let _ = cache.get(&1);
        cache.put(3, "c");
        assert_eq!(cache.get(&1), Some("a"));
        assert_eq!(cache.get(&2), None);
    }

    #[test]
    fn re_putting_existing_key_does_not_evict() {
        let cache: LruDiffCache<u32, &'static str> = LruDiffCache::new(2);
        cache.put(1, "a");
        cache.put(2, "b");
        cache.put(1, "a2");
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get(&1), Some("a2"));
        assert_eq!(cache.get(&2), Some("b"));
    }
}

/// The cache-invalidation table is encoded as functions over this enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CacheKey {
    Viewer,
    DashboardReviewRequested,
    DashboardAuthored,
    DashboardAssigned,
    PrList(crate::data::models::Repo),
    PrDetail(crate::data::models::PrId),
    PrFiles(crate::data::models::PrId, String), // head_sha
    PrDiff(crate::data::models::PrId, String),  // head_sha
    PrChecks(crate::data::models::PrId),
}

impl CacheKey {
    #[must_use]
    pub fn is_dashboard(&self) -> bool {
        matches!(
            self,
            CacheKey::DashboardReviewRequested
                | CacheKey::DashboardAuthored
                | CacheKey::DashboardAssigned
        )
    }

    #[must_use]
    pub fn is_pr_list_of(&self, repo: &crate::data::models::Repo) -> bool {
        matches!(self, CacheKey::PrList(r) if r == repo)
    }

    #[must_use]
    pub fn is_pr_resource_of(&self, id: &crate::data::models::PrId) -> bool {
        match self {
            CacheKey::PrDetail(p)
            | CacheKey::PrFiles(p, _)
            | CacheKey::PrDiff(p, _)
            | CacheKey::PrChecks(p) => p == id,
            _ => false,
        }
    }
}

#[cfg(test)]
mod key_tests {
    use super::*;
    use crate::data::models::{PrId, Repo};

    #[test]
    fn dashboard_keys_recognised() {
        assert!(CacheKey::DashboardReviewRequested.is_dashboard());
        assert!(CacheKey::DashboardAuthored.is_dashboard());
        assert!(CacheKey::DashboardAssigned.is_dashboard());
        assert!(!CacheKey::Viewer.is_dashboard());
    }

    #[test]
    fn pr_resource_match_is_per_id() {
        let id = PrId {
            repo: Repo {
                owner: "a".into(),
                name: "b".into(),
            },
            number: 1,
        };
        let other = PrId {
            repo: Repo {
                owner: "a".into(),
                name: "b".into(),
            },
            number: 2,
        };
        assert!(CacheKey::PrDetail(id.clone()).is_pr_resource_of(&id));
        assert!(!CacheKey::PrDetail(id.clone()).is_pr_resource_of(&other));
    }
}

/// Every write action returns the list of cache predicates that must be invalidated.
/// This is the executable form of the cache-invalidation table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteKind {
    SubmitReview { id: crate::data::models::PrId },
    PostPrComment { id: crate::data::models::PrId },
    PostLineComment { id: crate::data::models::PrId },
    ReplyToThread { id: crate::data::models::PrId },
    Merge { id: crate::data::models::PrId },
    Close { id: crate::data::models::PrId },
    Reopen { id: crate::data::models::PrId },
}

impl WriteKind {
    /// The PR this write targets. Every variant carries exactly one `id`.
    #[must_use]
    pub fn pr_id(&self) -> &crate::data::models::PrId {
        match self {
            WriteKind::SubmitReview { id }
            | WriteKind::PostPrComment { id }
            | WriteKind::PostLineComment { id }
            | WriteKind::ReplyToThread { id }
            | WriteKind::Merge { id }
            | WriteKind::Close { id }
            | WriteKind::Reopen { id } => id,
        }
    }
}

/// Predicate over `CacheKey`. Returns `true` when the key should be invalidated.
pub type InvalidationPredicate = Box<dyn Fn(&CacheKey) -> bool + Send + Sync>;

#[must_use]
pub fn invalidations_for(write: &WriteKind) -> Vec<InvalidationPredicate> {
    use WriteKind as W;
    fn dash_all() -> InvalidationPredicate {
        Box::new(|k: &CacheKey| k.is_dashboard())
    }
    fn dash_review_and_assigned() -> InvalidationPredicate {
        Box::new(|k: &CacheKey| {
            matches!(
                k,
                CacheKey::DashboardReviewRequested | CacheKey::DashboardAssigned
            )
        })
    }
    fn pr_detail_only(id: crate::data::models::PrId) -> InvalidationPredicate {
        Box::new(move |k: &CacheKey| matches!(k, CacheKey::PrDetail(p) if p == &id))
    }
    fn pr_all(id: crate::data::models::PrId) -> InvalidationPredicate {
        Box::new(move |k: &CacheKey| k.is_pr_resource_of(&id))
    }
    fn list(repo: crate::data::models::Repo) -> InvalidationPredicate {
        Box::new(move |k: &CacheKey| k.is_pr_list_of(&repo))
    }
    match write {
        // Submit review → pr_detail + review-requested + assigned + pr_list.
        W::SubmitReview { id } => vec![
            pr_detail_only(id.clone()),
            dash_review_and_assigned(),
            list(id.repo.clone()),
        ],
        // PR/line/thread comments → pr_detail only.
        W::PostPrComment { id } | W::PostLineComment { id } | W::ReplyToThread { id } => {
            vec![pr_detail_only(id.clone())]
        }
        // Merge → pr_detail + pr_files + pr_checks + dashboard:* + pr_list.
        W::Merge { id } => vec![pr_all(id.clone()), dash_all(), list(id.repo.clone())],
        // Close / reopen → pr_detail + dashboard:* + pr_list.
        W::Close { id } | W::Reopen { id } => vec![
            pr_detail_only(id.clone()),
            dash_all(),
            list(id.repo.clone()),
        ],
    }
}

#[cfg(test)]
mod invalidation_tests {
    use super::*;
    use crate::data::models::{PrId, Repo};

    fn pr_id() -> PrId {
        PrId {
            repo: Repo {
                owner: "a".into(),
                name: "b".into(),
            },
            number: 1,
        }
    }

    fn other_repo_list() -> CacheKey {
        CacheKey::PrList(Repo {
            owner: "x".into(),
            name: "y".into(),
        })
    }

    #[test]
    fn submit_review_invalidates_pr_detail_review_and_assigned_dashboards_and_repo_list() {
        let id = pr_id();
        let preds = invalidations_for(&WriteKind::SubmitReview { id: id.clone() });
        let pr_detail = CacheKey::PrDetail(id.clone());
        let review_dash = CacheKey::DashboardReviewRequested;
        let assigned_dash = CacheKey::DashboardAssigned;
        let authored_dash = CacheKey::DashboardAuthored;
        assert!(preds.iter().any(|p| p(&pr_detail)));
        assert!(preds.iter().any(|p| p(&review_dash)));
        assert!(preds.iter().any(|p| p(&assigned_dash)));
        assert!(
            !preds.iter().any(|p| p(&authored_dash)),
            "submit review does NOT touch dashboard:authored"
        );
        assert!(!preds.iter().any(|p| p(&other_repo_list())));
    }

    #[test]
    fn post_pr_comment_invalidates_only_pr_detail() {
        let id = pr_id();
        let preds = invalidations_for(&WriteKind::PostPrComment { id: id.clone() });
        assert!(preds.iter().any(|p| p(&CacheKey::PrDetail(id.clone()))));
        for k in [
            CacheKey::DashboardReviewRequested,
            CacheKey::DashboardAuthored,
            CacheKey::DashboardAssigned,
            CacheKey::PrList(id.repo.clone()),
            CacheKey::PrFiles(id.clone(), "sha".into()),
            CacheKey::PrChecks(id.clone()),
        ] {
            assert!(
                !preds.iter().any(|p| p(&k)),
                "comments must NOT invalidate {k:?}"
            );
        }
    }

    #[test]
    fn post_line_comment_and_reply_only_pr_detail() {
        let id = pr_id();
        for write in [
            WriteKind::PostLineComment { id: id.clone() },
            WriteKind::ReplyToThread { id: id.clone() },
        ] {
            let preds = invalidations_for(&write);
            assert!(preds.iter().any(|p| p(&CacheKey::PrDetail(id.clone()))));
            assert!(!preds.iter().any(|p| p(&CacheKey::DashboardAuthored)));
            assert!(!preds.iter().any(|p| p(&CacheKey::PrList(id.repo.clone()))));
        }
    }

    #[test]
    fn merge_invalidates_all_pr_resources_all_dashboards_and_repo_list() {
        let id = pr_id();
        let preds = invalidations_for(&WriteKind::Merge { id: id.clone() });
        for key in [
            CacheKey::PrDetail(id.clone()),
            CacheKey::PrFiles(id.clone(), "sha".into()),
            CacheKey::PrDiff(id.clone(), "sha".into()),
            CacheKey::PrChecks(id.clone()),
            CacheKey::DashboardReviewRequested,
            CacheKey::DashboardAuthored,
            CacheKey::DashboardAssigned,
            CacheKey::PrList(id.repo.clone()),
        ] {
            assert!(
                preds.iter().any(|p| p(&key)),
                "merge must invalidate {key:?}"
            );
        }
        assert!(!preds.iter().any(|p| p(&other_repo_list())));
    }

    #[test]
    fn close_invalidates_pr_detail_all_dashboards_and_repo_list_not_files() {
        let id = pr_id();
        let preds = invalidations_for(&WriteKind::Close { id: id.clone() });
        assert!(preds.iter().any(|p| p(&CacheKey::PrDetail(id.clone()))));
        assert!(preds.iter().any(|p| p(&CacheKey::DashboardReviewRequested)));
        assert!(preds.iter().any(|p| p(&CacheKey::PrList(id.repo.clone()))));
        assert!(
            !preds
                .iter()
                .any(|p| p(&CacheKey::PrFiles(id.clone(), "sha".into()))),
            "close does NOT invalidate pr_files"
        );
        assert!(
            !preds.iter().any(|p| p(&CacheKey::PrChecks(id.clone()))),
            "close does NOT invalidate pr_checks"
        );
    }
}
