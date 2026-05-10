//! Lightweight in-process API rate limiting.
//!
//! This is intentionally simple for the first server hardening pass. It limits
//! by bearer token when present (authenticated requests), then by the peer
//! socket IP extracted by Axum `ConnectInfo`, and finally by an anonymous
//! bucket for clients that provide neither.
//!
//! Security note: for internet-facing deployments we intentionally do **not**
//! trust `X-Forwarded-For` / `X-Real-IP` request headers since clients can
//! spoof them when the app is reachable directly.

use std::{
    collections::{hash_map::DefaultHasher, HashMap, VecDeque},
    hash::{Hash, Hasher},
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    extract::{ConnectInfo, Request, State},
    http,
    middleware::Next,
    response::Response,
};
use tokio::sync::Mutex;

use crate::{
    error::{AppError, Result},
    state::AppState,
};

#[derive(Debug)]
pub struct RateLimiter {
    max_requests: usize,
    window: Duration,
    max_buckets: usize,
    buckets: Mutex<HashMap<String, VecDeque<Instant>>>,
}

const DEFAULT_MAX_BUCKETS: usize = 100_000;

impl RateLimiter {
    pub fn new(max_requests: u32, window: Duration) -> Self {
        Self::with_max_buckets(max_requests, window, DEFAULT_MAX_BUCKETS)
    }

    pub fn with_max_buckets(max_requests: u32, window: Duration, max_buckets: usize) -> Self {
        Self {
            max_requests: max_requests.max(1) as usize,
            window,
            max_buckets: max_buckets.max(1),
            buckets: Mutex::new(HashMap::new()),
        }
    }

    pub async fn allow(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut buckets = self.buckets.lock().await;

        self.prune_expired(&mut buckets, now);
        if !buckets.contains_key(key) && buckets.len() >= self.max_buckets {
            Self::evict_oldest_bucket(&mut buckets);
        }

        let bucket = buckets.entry(key.to_owned()).or_default();

        if bucket.len() >= self.max_requests {
            return false;
        }

        bucket.push_back(now);
        true
    }

    fn prune_expired(&self, buckets: &mut HashMap<String, VecDeque<Instant>>, now: Instant) {
        buckets.retain(|_, bucket| {
            while bucket
                .front()
                .is_some_and(|oldest| now.duration_since(*oldest) >= self.window)
            {
                bucket.pop_front();
            }
            !bucket.is_empty()
        });
    }

    fn evict_oldest_bucket(buckets: &mut HashMap<String, VecDeque<Instant>>) {
        if let Some(oldest_key) = buckets
            .iter()
            .filter_map(|(key, bucket)| bucket.back().map(|last| (key.clone(), *last)))
            .min_by_key(|(_, last)| *last)
            .map(|(key, _)| key)
        {
            buckets.remove(&oldest_key);
        }
    }
}

pub type SharedRateLimiter = Arc<RateLimiter>;

pub async fn enforce(State(state): State<AppState>, req: Request, next: Next) -> Result<Response> {
    let key = request_key(&req);
    if !state.rate_limiter.allow(&key).await {
        return Err(AppError::RateLimited(
            "too many requests; slow down and retry shortly".into(),
        ));
    }

    Ok(next.run(req).await)
}

fn request_key<B>(req: &http::Request<B>) -> String {
    if let Some(token) = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .strip_prefix("Bearer ")
                .or_else(|| value.strip_prefix("bearer "))
        })
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        let mut hasher = DefaultHasher::new();
        token.hash(&mut hasher);
        return format!("bearer:{:016x}", hasher.finish());
    }

    if let Some(connect_info) = req.extensions().get::<ConnectInfo<SocketAddr>>() {
        return format!("ip:{}", connect_info.0.ip());
    }

    "anonymous".into()
}

#[cfg(test)]
mod tests {
    use super::{request_key, RateLimiter};
    use axum::extract::ConnectInfo;
    use axum::http::{header, Request};
    use std::net::SocketAddr;
    use std::time::Duration;

    #[tokio::test]
    async fn limiter_rejects_after_window_capacity() {
        let limiter = RateLimiter::new(2, Duration::from_secs(60));

        assert!(limiter.allow("client").await);
        assert!(limiter.allow("client").await);
        assert!(!limiter.allow("client").await);
    }

    #[tokio::test]
    async fn limiter_caps_bucket_count() {
        let limiter = RateLimiter::with_max_buckets(1, Duration::from_secs(60), 2);

        assert!(limiter.allow("a").await);
        assert!(limiter.allow("b").await);
        assert!(limiter.allow("c").await);

        let buckets = limiter.buckets.lock().await;
        assert!(buckets.len() <= 2);
    }

    #[test]
    fn request_key_uses_peer_ip_from_connect_info() {
        let mut req = Request::builder().body(()).unwrap();
        req.extensions_mut().insert(ConnectInfo(SocketAddr::from((
            [203, 0, 113, 1],
            43210,
        ))));

        assert_eq!(request_key(&req), "ip:203.0.113.1");
    }

    #[test]
    fn request_key_prefers_bearer_over_ip_identity() {
        let mut req = Request::builder()
            .header(header::AUTHORIZATION, "Bearer secret-token")
            .body(())
            .unwrap();
        req.extensions_mut().insert(ConnectInfo(SocketAddr::from((
            [203, 0, 113, 1],
            12345,
        ))));

        assert!(request_key(&req).starts_with("bearer:"));
    }

    #[test]
    fn request_key_ignores_untrusted_forwarded_headers() {
        let req = Request::builder()
            .header("x-forwarded-for", "203.0.113.1, 10.0.0.1")
            .body(())
            .unwrap();

        assert_eq!(request_key(&req), "anonymous");
    }

    #[test]
    fn request_key_hashes_bearer_token() {
        let req = Request::builder()
            .header(header::AUTHORIZATION, "Bearer secret-token")
            .body(())
            .unwrap();

        assert!(request_key(&req).starts_with("bearer:"));
        assert!(!request_key(&req).contains("secret-token"));
    }
}
