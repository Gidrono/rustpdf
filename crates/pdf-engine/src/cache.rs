use std::collections::HashMap;
use std::num::NonZeroUsize;

use doxo_document::{DocumentId, PageId};
use serde::{Deserialize, Serialize};

/// Cache key for a rendered page (or tile) at a quantized zoom + DPR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TileKey {
    pub document_id: DocumentId,
    pub page: PageId,
    /// Zoom quantized to centi-zoom (e.g. 1.25 → 125) to avoid float key churn.
    pub zoom_centi: u32,
    /// `pixels_per_point` quantized to centi (e.g. 2.0 → 200).
    pub ppp_centi: u32,
}

impl TileKey {
    pub fn new(document_id: DocumentId, page: PageId, zoom: f32, pixels_per_point: f32) -> Self {
        Self {
            document_id,
            page,
            zoom_centi: (zoom * 100.0).round().max(1.0) as u32,
            ppp_centi: (pixels_per_point * 100.0).round().max(50.0) as u32,
        }
    }

    pub fn zoom(self) -> f32 {
        self.zoom_centi as f32 / 100.0
    }

    pub fn pixels_per_point(self) -> f32 {
        self.ppp_centi as f32 / 100.0
    }
}

#[derive(Debug, Clone)]
pub struct CachedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Default, Clone)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub entries: usize,
    pub bytes: usize,
}

/// Simple LRU-ish tile cache (HashMap + insertion order eviction).
///
/// Not a full LRU list — Phase 1 priority is correctness and bounded memory.
/// TODO(Phase 2): proper LRU + disk spill for huge documents.
#[derive(Debug)]
pub struct TileCache {
    max_entries: NonZeroUsize,
    map: HashMap<TileKey, CachedImage>,
    order: Vec<TileKey>,
    hits: u64,
    misses: u64,
}

impl TileCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            max_entries: NonZeroUsize::new(max_entries.max(1)).unwrap(),
            map: HashMap::new(),
            order: Vec::new(),
            hits: 0,
            misses: 0,
        }
    }

    pub fn get(&mut self, key: &TileKey) -> Option<&CachedImage> {
        if self.map.contains_key(key) {
            self.hits += 1;
            // Refresh order
            if let Some(pos) = self.order.iter().position(|k| k == key) {
                let k = self.order.remove(pos);
                self.order.push(k);
            }
            self.map.get(key)
        } else {
            self.misses += 1;
            None
        }
    }

    pub fn insert(&mut self, key: TileKey, image: CachedImage) {
        if self.map.contains_key(&key) {
            self.map.insert(key, image);
            return;
        }
        while self.map.len() >= self.max_entries.get() {
            if let Some(old) = self.order.first().copied() {
                self.order.remove(0);
                self.map.remove(&old);
            } else {
                break;
            }
        }
        self.order.push(key);
        self.map.insert(key, image);
    }

    pub fn clear_document(&mut self, document_id: DocumentId) {
        self.order.retain(|k| k.document_id != document_id);
        self.map.retain(|k, _| k.document_id != document_id);
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }

    pub fn stats(&self) -> CacheStats {
        let bytes = self.map.values().map(|i| i.rgba.len()).sum();
        CacheStats {
            hits: self.hits,
            misses: self.misses,
            entries: self.map.len(),
            bytes,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThumbnailKey {
    pub document_id: DocumentId,
    pub page: PageId,
}

/// Thumbnail sidebar cache (separate from zoomed page tiles).
#[derive(Debug)]
pub struct ThumbnailCache {
    inner: HashMap<ThumbnailKey, CachedImage>,
    max_entries: usize,
}

impl ThumbnailCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            inner: HashMap::new(),
            max_entries: max_entries.max(1),
        }
    }

    pub fn get(&self, key: &ThumbnailKey) -> Option<&CachedImage> {
        self.inner.get(key)
    }

    pub fn insert(&mut self, key: ThumbnailKey, image: CachedImage) {
        if self.inner.len() >= self.max_entries && !self.inner.contains_key(&key) {
            // Drop an arbitrary entry — fine for Phase 1.
            if let Some(victim) = self.inner.keys().next().copied() {
                self.inner.remove(&victim);
            }
        }
        self.inner.insert(key, image);
    }

    pub fn clear_document(&mut self, document_id: DocumentId) {
        self.inner.retain(|k, _| k.document_id != document_id);
    }

    pub fn clear(&mut self) {
        self.inner.clear();
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }
}
