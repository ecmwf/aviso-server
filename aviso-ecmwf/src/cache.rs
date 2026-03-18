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
