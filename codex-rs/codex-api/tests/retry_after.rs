#![allow(clippy::expect_used)]
//! #510: the transport retry of an HTTP 429 (Z.AI's `retry_429` policy)
//! honours `Retry-After`. A short one is waited out; a long one ends the
//! retries at once, so the provider-request throttle owns that wait.
use std::time::Duration;
use std::time::Instant;

use codex_api::RetryConfig;
use codex_client::HttpTransport;
use codex_client::Request;
use codex_client::ReqwestTransport;
use codex_client::TransportError;
use codex_client::run_with_retry;
use codex_http_client::HttpClientBuilder;
use http::Method;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;

/// Z.AI's transport retry policy (`ModelProviderInfo::to_api_provider`).
fn zai_retry() -> RetryConfig {
    RetryConfig {
        max_attempts: 4,
        base_delay: Duration::from_millis(10),
        retry_429: true,
        retry_5xx: true,
        retry_transport: true,
    }
}

/// Serves `first` once, then 200 for every later request.
async fn server_answering(first: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(first)
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    server
}

/// Sends one request under the Z.AI policy: (result status, requests the
/// server saw, time taken).
async fn send(server: &MockServer) -> (Result<u16, TransportError>, usize, Duration) {
    let transport = ReqwestTransport::from_http_client(
        HttpClientBuilder::new()
            .build_direct()
            .expect("test HTTP client should build"),
    );
    let url = format!("{}/chat/completions", server.uri());
    let started = Instant::now();
    let result = run_with_retry(
        zai_retry().to_policy(),
        || Request::new(Method::POST, url.clone()),
        |request, _attempt| transport.execute(request),
    )
    .await
    .map(|response| response.status.as_u16());
    let seen = server.received_requests().await.unwrap_or_default().len();
    (result, seen, started.elapsed())
}

fn rate_limited() -> ResponseTemplate {
    ResponseTemplate::new(429).set_body_string(r#"{"error":{"code":"1302"}}"#)
}

#[tokio::test]
async fn a_long_retry_after_ends_transport_retries_at_once() {
    let server = server_answering(rate_limited().insert_header("retry-after", "120")).await;
    let (result, seen, elapsed) = send(&server).await;
    let Err(TransportError::Http {
        status, headers, ..
    }) = result
    else {
        panic!("expected the 429 back, got {result:?}");
    };
    assert_eq!(status.as_u16(), 429);
    // The caller's throttle reads the provider's wait from the 429 it gets.
    assert_eq!(
        headers
            .as_ref()
            .and_then(|headers| headers.get("retry-after"))
            .and_then(|value| value.to_str().ok()),
        Some("120")
    );
    assert_eq!(seen, 1, "a 429 asking for 120 s was retried");
    assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
}

#[tokio::test]
async fn an_http_date_retry_after_ends_transport_retries_at_once() {
    let server = server_answering(
        rate_limited().insert_header("retry-after", "Wed, 21 Oct 2099 07:28:00 GMT"),
    )
    .await;
    let (result, seen, _) = send(&server).await;
    assert!(
        matches!(result, Err(TransportError::Http { .. })),
        "{result:?}"
    );
    assert_eq!(seen, 1);
}

#[tokio::test]
async fn a_short_retry_after_is_waited_out_before_the_retry() {
    let server = server_answering(rate_limited().insert_header("retry-after", "1")).await;
    let (result, seen, elapsed) = send(&server).await;
    assert_eq!(result.ok(), Some(200));
    assert_eq!(seen, 2);
    assert!(
        elapsed >= Duration::from_secs(1),
        "retried after {elapsed:?}"
    );
}

#[tokio::test]
async fn retry_after_ms_is_honoured_too() {
    let server = server_answering(
        rate_limited()
            .insert_header("retry-after-ms", "60000")
            .insert_header("retry-after", "1"),
    )
    .await;
    let (result, seen, _) = send(&server).await;
    assert!(
        matches!(result, Err(TransportError::Http { .. })),
        "{result:?}"
    );
    assert_eq!(seen, 1);
}

/// A transient overload without `Retry-After` keeps the bounded backoff retry
/// it was enabled for (Z.AI code 1305).
#[tokio::test]
async fn a_429_without_retry_after_keeps_the_backoff_retry() {
    let server = server_answering(rate_limited()).await;
    let (result, seen, _) = send(&server).await;
    assert_eq!(result.ok(), Some(200));
    assert_eq!(seen, 2);
}
