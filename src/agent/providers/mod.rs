pub mod anthropic;
pub mod openai_compat;
pub mod replay;
pub mod text_action_parser;

pub use anthropic::AnthropicProvider;
pub use openai_compat::OpenAiCompatibleProvider;
pub use replay::ReplayProvider;

use std::time::Duration;

use crate::agent::provider::ProviderError;

/// Per-request HTTP timeout for the built-in HTTP providers. The Cloudflare
/// gateway in front of the shared proxy gives up around ~100s and returns a
/// 524; normal upstream responses land well under 35s, so a 60s client timeout
/// cuts hung connections ~40s earlier while leaving comfortable headroom.
pub(crate) const HTTP_TIMEOUT: Duration = Duration::from_secs(60);

/// Builds an HTTP client with [`HTTP_TIMEOUT`], falling back to the default
/// client if the builder ever fails.
pub(crate) fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Sends an HTTP request, retrying on transient failures: client timeouts,
/// connect errors, and 5xx upstream responses (the 524 gateway timeout we see
/// during network instability). Retries up to 2 times (3 attempts total) with a
/// short linear backoff.
///
/// 4xx responses are returned as-is so callers can keep their existing mapping
/// of 401/403 → AuthError and 429 → RateLimited. A 5xx that survives all
/// attempts is also returned as-is, letting the caller surface the upstream
/// body text in its NetworkError.
pub(crate) async fn send_with_retry(
    request: reqwest::RequestBuilder,
) -> Result<reqwest::Response, ProviderError> {
    const MAX_ATTEMPTS: u32 = 3;
    let mut last_err: Option<String> = None;
    for attempt in 1..=MAX_ATTEMPTS {
        // Clone so the builder survives for the next attempt. JSON bodies are
        // always cloneable; if cloning ever fails, fall back to a single shot.
        let Some(req) = request.try_clone() else {
            return request
                .send()
                .await
                .map_err(|e| ProviderError::NetworkError(e.to_string()));
        };
        match req.send().await {
            Ok(resp) => {
                if resp.status().is_server_error() && attempt < MAX_ATTEMPTS {
                    last_err = Some(format!("upstream status {}", resp.status()));
                    tokio::time::sleep(retry_backoff(attempt)).await;
                    continue;
                }
                return Ok(resp);
            }
            Err(e) => {
                let transient = e.is_timeout() || e.is_connect();
                if transient && attempt < MAX_ATTEMPTS {
                    last_err = Some(e.to_string());
                    tokio::time::sleep(retry_backoff(attempt)).await;
                    continue;
                }
                return Err(ProviderError::NetworkError(e.to_string()));
            }
        }
    }
    Err(ProviderError::NetworkError(last_err.unwrap_or_else(|| {
        "request failed after retries".to_string()
    })))
}

fn retry_backoff(attempt: u32) -> Duration {
    Duration::from_millis(500 * attempt as u64)
}
