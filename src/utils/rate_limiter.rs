use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::time::{Duration, Instant};
use std::collections::HashMap;
use tokio::sync::Mutex;

/// Rate limiter for API calls
pub struct RateLimiter {
    /// Semaphore for concurrent request limiting
    semaphore: Arc<Semaphore>,
    /// Track last request time per key
    last_request: Arc<Mutex<HashMap<String, Instant>>>,
    /// Minimum time between requests (per key)
    min_interval: Duration,
}

impl RateLimiter {
    pub fn new(max_concurrent: usize, min_interval_ms: u64) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            last_request: Arc::new(Mutex::new(HashMap::new())),
            min_interval: Duration::from_millis(min_interval_ms),
        }
    }

    /// Acquire a permit for making a request
    pub async fn acquire(&self, key: String) -> Result<RateLimitPermit, String> {
        // First, acquire semaphore permit for concurrent limiting
        let permit = self.semaphore.clone().acquire_owned().await
            .map_err(|_| "Failed to acquire semaphore permit".to_string())?;

        // Then check rate limit per key
        let mut last_request_map = self.last_request.lock().await;
        let now = Instant::now();
        
        if let Some(&last_time) = last_request_map.get(&key) {
            let elapsed = now.duration_since(last_time);
            if elapsed < self.min_interval {
                let wait_time = self.min_interval - elapsed;
                drop(last_request_map); // Release lock during sleep
                tokio::time::sleep(wait_time).await;
                
                // Re-acquire lock and update time
                let mut last_request_map = self.last_request.lock().await;
                last_request_map.insert(key.clone(), Instant::now());
            } else {
                last_request_map.insert(key.clone(), now);
            }
        } else {
            last_request_map.insert(key.clone(), now);
        }

        Ok(RateLimitPermit {
            _permit: permit,
            key,
            last_request: self.last_request.clone(),
        })
    }
}

/// RAII guard for rate limit permit
pub struct RateLimitPermit {
    _permit: tokio::sync::OwnedSemaphorePermit,
    key: String,
    last_request: Arc<Mutex<HashMap<String, Instant>>>,
}

impl Drop for RateLimitPermit {
    fn drop(&mut self) {
        // Could update last request time here if needed
    }
}

/// Global rate limiter for LLM API calls
pub fn create_llm_rate_limiter() -> RateLimiter {
    // Allow 5 concurrent requests, with minimum 200ms between requests per user
    RateLimiter::new(5, 200)
}

/// Global rate limiter for external API calls (web search, etc.)
pub fn create_api_rate_limiter() -> RateLimiter {
    // Allow 10 concurrent requests, with minimum 100ms between requests
    RateLimiter::new(10, 100)
}