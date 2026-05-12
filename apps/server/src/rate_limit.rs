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
mod tests;
