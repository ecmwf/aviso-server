use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::Instant;

struct CacheEntry {
    destinations: Vec<String>,
    expires_at: Instant,
}

pub struct DestinationCache {
    entries: RwLock<HashMap<String, CacheEntry>>,
    ttl: Duration,
}

impl DestinationCache {
    pub fn new(ttl_seconds: u64) -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            ttl: Duration::from_secs(ttl_seconds),
        }
    }

    pub async fn get(&self, username: &str) -> Option<Vec<String>> {
        let entries = self.entries.read().await;
        entries.get(username).and_then(|entry| {
            if Instant::now() < entry.expires_at {
                Some(entry.destinations.clone())
            } else {
                None
            }
        })
    }

    pub async fn set(&self, username: &str, destinations: Vec<String>) {
        let mut entries = self.entries.write().await;
        entries.insert(
            username.to_string(),
            CacheEntry {
                destinations,
                expires_at: Instant::now() + self.ttl,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cache_miss_returns_none() {
        let cache = DestinationCache::new(300);
        assert_eq!(cache.get("unknown_user").await, None);
    }

    #[tokio::test]
    async fn cache_hit_returns_stored_destinations() {
        let cache = DestinationCache::new(300);
        cache
            .set("john", vec!["CIP".to_string(), "FOO".to_string()])
            .await;
        let result = cache.get("john").await;
        assert_eq!(result, Some(vec!["CIP".to_string(), "FOO".to_string()]));
    }

    #[tokio::test]
    async fn cache_entry_expires_after_ttl() {
        let cache = DestinationCache::new(1); // 1 second TTL
        cache.set("john", vec!["CIP".to_string()]).await;
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        assert_eq!(cache.get("john").await, None);
    }

    #[tokio::test]
    async fn cache_overwrite_replaces_value() {
        let cache = DestinationCache::new(300);
        cache.set("john", vec!["CIP".to_string()]).await;
        cache.set("john", vec!["FOO".to_string()]).await;
        assert_eq!(cache.get("john").await, Some(vec!["FOO".to_string()]));
    }

    #[tokio::test]
    async fn cache_independent_entries_per_user() {
        let cache = DestinationCache::new(300);
        cache.set("alice", vec!["CIP".to_string()]).await;
        cache.set("bob", vec!["FOO".to_string()]).await;
        assert_eq!(cache.get("alice").await, Some(vec!["CIP".to_string()]));
        assert_eq!(cache.get("bob").await, Some(vec!["FOO".to_string()]));
    }
}
