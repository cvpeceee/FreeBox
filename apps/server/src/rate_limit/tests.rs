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

#[tokio::test]
async fn concurrent_requests_do_not_corrupt_state() {
    use std::sync::Arc;

    let limiter = Arc::new(RateLimiter::new(50, Duration::from_secs(60)));
    let mut handles = Vec::new();

    for _ in 0..10 {
        let l = limiter.clone();
        handles.push(tokio::spawn(async move {
            for _ in 0..5 {
                l.allow("shared-client").await;
            }
        }));
    }

    for h in handles {
        h.await.expect("task must not panic");
    }

    // After 10 × 5 = 50 requests with a limit of 50, the bucket is full.
    let allowed = limiter.allow("shared-client").await;
    assert!(!allowed, "rate limiter must deny after limit exceeded");
}

#[tokio::test]
async fn independent_clients_have_separate_buckets() {
    let limiter = RateLimiter::new(1, Duration::from_secs(60));

    assert!(limiter.allow("client-a").await, "first request for A must be allowed");
    assert!(!limiter.allow("client-a").await, "second request for A must be denied");

    // Client B has its own independent bucket.
    assert!(limiter.allow("client-b").await, "first request for B must be allowed");
}
