use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;

use crate::config::home_dir;
use crate::llm::{LlmResponse, Message, Usage};

// ── Response cache ──────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
pub struct CacheEntry {
    pub content: String,
    usage: CacheUsage,
    ts: u64,
}

#[derive(Serialize, Deserialize, Clone, Default)]
struct CacheUsage {
    input_tokens: u32,
    output_tokens: u32,
    cache_read: u32,
    cache_creation: u32,
    total_tokens: u32,
}

pub struct ResponseCache {
    entries: HashMap<String, CacheEntry>,
    max_size: usize,
    path: PathBuf,
    dirty: bool,
}

pub fn cache_key(model: &str, system: &str, messages: &[Message]) -> String {
    let mut h = DefaultHasher::new();
    model.hash(&mut h);
    system.hash(&mut h);
    for m in messages {
        m.role.hash(&mut h);
        m.content.hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

impl ResponseCache {
    /// Load the on-disk cache. `max_size == 0` disables caching entirely
    /// (`put` becomes a no-op and nothing is ever written back).
    pub fn load(max_size: usize) -> Self {
        let home = home_dir().unwrap_or_default();
        let path = PathBuf::from(&home).join(".local/bin/,.cache.json");
        let entries = std::fs::read_to_string(&path)
            .ok()
            .and_then(|data| serde_json::from_str::<HashMap<String, CacheEntry>>(&data).ok())
            .unwrap_or_default();
        Self {
            entries,
            max_size,
            path,
            dirty: false,
        }
    }

    pub fn get(&self, key: &str) -> Option<&CacheEntry> {
        self.entries.get(key)
    }

    /// Insert an entry, evicting the oldest entry when over capacity.
    /// No-op when `max_size == 0` (caching disabled).
    pub fn put(&mut self, key: String, entry: CacheEntry) {
        if self.max_size == 0 {
            return;
        }
        self.entries.insert(key, entry);
        self.dirty = true;
        // Evict oldest if over capacity
        if self.entries.len() > self.max_size {
            let mut oldest_key = String::new();
            let mut oldest_ts = u64::MAX;
            for (k, v) in &self.entries {
                if v.ts < oldest_ts {
                    oldest_ts = v.ts;
                    oldest_key = k.clone();
                }
            }
            if !oldest_key.is_empty() {
                self.entries.remove(&oldest_key);
            }
        }
    }

    /// Persist the cache if dirty. Writes to a temp file in the same
    /// directory first, then renames over the target so a crash mid-write
    /// cannot leave a corrupt cache file. Best-effort: errors are ignored.
    pub fn save(&self) {
        if !self.dirty {
            return;
        }
        if let Ok(json) = serde_json::to_string(&self.entries) {
            let mut tmp = self.path.clone().into_os_string();
            tmp.push(".tmp");
            let tmp = PathBuf::from(tmp);
            if std::fs::write(&tmp, json).is_ok() && std::fs::rename(&tmp, &self.path).is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Clear all entries in memory. The on-disk file is untouched until
    /// the next `save()` (which will write an empty map if no new entries
    /// are inserted before then).
    pub fn clear(&mut self) {
        if !self.entries.is_empty() {
            self.entries.clear();
            self.dirty = true;
        }
    }
}

fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl From<&LlmResponse> for CacheEntry {
    fn from(resp: &LlmResponse) -> Self {
        Self {
            content: resp.content.clone(),
            usage: CacheUsage {
                input_tokens: resp.usage.input_tokens,
                output_tokens: resp.usage.output_tokens,
                cache_read: resp.usage.cache_read,
                cache_creation: resp.usage.cache_creation,
                total_tokens: resp.usage.total_tokens,
            },
            ts: now_ts(),
        }
    }
}

impl CacheEntry {
    pub fn to_response(&self) -> LlmResponse {
        LlmResponse {
            content: self.content.clone(),
            usage: Usage {
                input_tokens: self.usage.input_tokens,
                output_tokens: self.usage.output_tokens,
                cache_read: self.usage.cache_read,
                cache_creation: self.usage.cache_creation,
                total_tokens: self.usage.total_tokens,
                duration_ms: 0,
                from_cache: true,
            },
            cache_key: None,
        }
    }
}
