//! Memory-bounded LRU cache of rendered animation frames, shared across streams.
//! The dispatcher caches looping sequences that meet the minimum frame count.

use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use parking_lot::Mutex;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    pub url: String,
    pub width: u32,
    pub height: u32,
    pub fit: &'static str,
    pub method: &'static str,
}

#[derive(Debug, Clone)]
pub struct CachedSequence {
    /// Rendered RGB888 frames at target size, paired with delays in milliseconds.
    pub frames: Vec<(Bytes, f32)>,
    bytes: usize,
}

impl CachedSequence {
    pub fn new(frames: Vec<(Bytes, f32)>) -> Self {
        let bytes: usize = frames.iter().map(|(b, _)| b.len()).sum();
        Self { frames, bytes }
    }
}

#[derive(Debug, Default)]
struct Inner {
    lru: Vec<CacheKey>, // most-recent at the back
    entries: HashMap<CacheKey, Arc<CachedSequence>>,
    bytes: usize,
}

#[derive(Debug, Clone)]
pub struct FrameCache {
    inner: Arc<Mutex<Inner>>,
    max_bytes: usize,
    min_frames: usize,
}

impl FrameCache {
    pub fn new(max_mb: u32, min_frames: u32) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            max_bytes: (max_mb as usize) * 1024 * 1024,
            min_frames: min_frames as usize,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.max_bytes > 0
    }

    pub fn eligible(&self, frame_count: usize, looping: bool) -> bool {
        self.is_enabled() && looping && frame_count >= self.min_frames
    }

    /// Looks up cached frames; touches LRU on hit.
    pub fn get(&self, key: &CacheKey) -> Option<Arc<CachedSequence>> {
        let mut inner = self.inner.lock();
        let seq = inner.entries.get(key).cloned()?;
        if let Some(pos) = inner.lru.iter().position(|k| k == key) {
            let k = inner.lru.remove(pos);
            inner.lru.push(k);
        }
        Some(seq)
    }

    /// Inserts a sequence. Evicts LRU entries until the new total fits under
    /// `max_bytes`. If `seq.bytes > max_bytes` the insert is refused.
    pub fn insert(&self, key: CacheKey, seq: CachedSequence) -> Option<Arc<CachedSequence>> {
        if seq.bytes > self.max_bytes {
            return None;
        }
        let arc = Arc::new(seq);
        let mut inner = self.inner.lock();

        if let Some(old) = inner.entries.remove(&key) {
            inner.bytes = inner.bytes.saturating_sub(old.bytes);
            if let Some(pos) = inner.lru.iter().position(|k| k == &key) {
                inner.lru.remove(pos);
            }
        }

        while inner.bytes + arc.bytes > self.max_bytes && !inner.lru.is_empty() {
            let evict = inner.lru.remove(0);
            if let Some(e) = inner.entries.remove(&evict) {
                inner.bytes = inner.bytes.saturating_sub(e.bytes);
            }
        }

        inner.bytes += arc.bytes;
        inner.entries.insert(key.clone(), arc.clone());
        inner.lru.push(key);
        Some(arc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(url: &str) -> CacheKey {
        CacheKey {
            url: url.into(),
            width: 64,
            height: 64,
            fit: "pad",
            method: "lanczos",
        }
    }

    fn seq(size_kb: usize) -> CachedSequence {
        CachedSequence::new(vec![(Bytes::from(vec![0u8; size_kb * 1024]), 100.0)])
    }

    #[test]
    fn eligibility_requires_loop_and_min_frames() {
        let c = FrameCache::new(32, 5);
        assert!(!c.eligible(4, true));
        assert!(c.eligible(5, true));
        assert!(!c.eligible(5, false));
    }

    #[test]
    fn zero_mb_disables_cache() {
        let c = FrameCache::new(0, 5);
        assert!(!c.is_enabled());
        assert!(!c.eligible(10, true));
    }

    #[test]
    fn lru_evicts_oldest() {
        let c = FrameCache::new(2, 1); // 2 MB cap, three 1 MB entries
        let k1 = key("a");
        let k2 = key("b");
        let k3 = key("c");
        c.insert(k1.clone(), seq(1024));
        c.insert(k2.clone(), seq(1024));
        c.insert(k3.clone(), seq(1024)); // evicts k1
        assert!(c.get(&k1).is_none());
        assert!(c.get(&k2).is_some());
        assert!(c.get(&k3).is_some());
    }

    #[test]
    fn touching_moves_to_mru() {
        let c = FrameCache::new(2, 1);
        let k1 = key("a");
        let k2 = key("b");
        let k3 = key("c");
        c.insert(k1.clone(), seq(1024));
        c.insert(k2.clone(), seq(1024));
        let _ = c.get(&k1); // touch k1 to protect it from eviction
        c.insert(k3.clone(), seq(1024)); // evicts k2, not k1
        assert!(c.get(&k1).is_some());
        assert!(c.get(&k2).is_none());
    }

    #[test]
    fn oversized_insert_refused() {
        let c = FrameCache::new(1, 1);
        let k1 = key("big");
        assert!(c.insert(k1.clone(), seq(2 * 1024)).is_none());
        assert!(c.get(&k1).is_none());
    }
}
