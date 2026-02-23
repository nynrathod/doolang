//! HTTP Client Utilities — Shared across all OAuth providers
//!
//! Minimal, synchronous HTTP client for OAuth token exchange and user info requests.
//! Uses `minreq` for simplicity — no async runtime needed at the FFI boundary.
//!
//! ## Design decisions
//! - Synchronous: FFI boundary is `extern "C"` (synchronous)
//! - Minimal deps: minreq is tiny, handles HTTPS via rustls
//! - All status codes returned (no auto-error for 4xx/5xx) — caller decides
//! - URL encoding follows RFC 3986

use std::time::Duration;

/// HTTP request timeout — 30 seconds (generous for OAuth API calls)
const HTTP_TIMEOUT_SECS: u64 = 30;

// ============================================================================
// URL ENCODING — RFC 3986
// ============================================================================

/// Percent-encode a string per RFC 3986 (unreserved characters pass through).
///
/// Unreserved characters: A-Z a-z 0-9 - _ . ~
pub fn url_encode(s: &str) -> String {
    let mut encoded = String::with_capacity(s.len() * 2);
    for byte in s.bytes() {
        match byte {
            b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            _ => {
                encoded.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    encoded
}

/// Build a URL with query parameters (properly encoded).
pub fn build_url(base: &str, params: &[(&str, &str)]) -> String {
    if params.is_empty() {
        return base.to_string();
    }

    let query = params
        .iter()
        .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
        .collect::<Vec<_>>()
        .join("&");

    format!("{}?{}", base, query)
}

/// Build a form-encoded body string from key-value pairs.
fn build_form_body(params: &[(&str, &str)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

// ============================================================================
// HTTP REQUESTS — Synchronous, returns (status_code, body)
// ============================================================================

/// POST with form-encoded body. Returns (status_code, response_body).
///
/// Sets Content-Type: application/x-www-form-urlencoded and Accept: application/json.
pub fn post_form(url: &str, params: &[(&str, &str)]) -> Result<(u16, String), String> {
    let form_body = build_form_body(params);

    let response = minreq::post(url)
        .with_header("Content-Type", "application/x-www-form-urlencoded")
        .with_header("Accept", "application/json")
        .with_timeout(HTTP_TIMEOUT_SECS)
        .with_body(form_body)
        .send()
        .map_err(|e| format!("HTTP POST to {} failed: {}", url, e))?;

    let status = response.status_code as u16;
    let body = response
        .as_str()
        .map(|s| s.to_string())
        .map_err(|e| format!("Failed to read response body: {}", e))?;

    Ok((status, body))
}

/// POST with form-encoded body and custom Accept header.
///
/// Used by GitHub which requires explicit Accept: application/json.
pub fn post_form_with_accept(
    url: &str,
    params: &[(&str, &str)],
    accept: &str,
) -> Result<(u16, String), String> {
    let form_body = build_form_body(params);

    let response = minreq::post(url)
        .with_header("Content-Type", "application/x-www-form-urlencoded")
        .with_header("Accept", accept)
        .with_timeout(HTTP_TIMEOUT_SECS)
        .with_body(form_body)
        .send()
        .map_err(|e| format!("HTTP POST to {} failed: {}", url, e))?;

    let status = response.status_code as u16;
    let body = response
        .as_str()
        .map(|s| s.to_string())
        .map_err(|e| format!("Failed to read response body: {}", e))?;

    Ok((status, body))
}

/// GET with Bearer token authentication. Returns (status_code, response_body).
pub fn get_with_bearer(url: &str, token: &str) -> Result<(u16, String), String> {
    let response = minreq::get(url)
        .with_header("Authorization", &format!("Bearer {}", token))
        .with_header("Accept", "application/json")
        .with_timeout(HTTP_TIMEOUT_SECS)
        .send()
        .map_err(|e| format!("HTTP GET to {} failed: {}", url, e))?;

    let status = response.status_code as u16;
    let body = response
        .as_str()
        .map(|s| s.to_string())
        .map_err(|e| format!("Failed to read response body: {}", e))?;

    Ok((status, body))
}

/// GET with Bearer token and custom User-Agent header.
///
/// Required by GitHub API which mandates a User-Agent header.
pub fn get_with_bearer_and_ua(
    url: &str,
    token: &str,
    user_agent: &str,
) -> Result<(u16, String), String> {
    let response = minreq::get(url)
        .with_header("Authorization", &format!("Bearer {}", token))
        .with_header("Accept", "application/json")
        .with_header("User-Agent", user_agent)
        .with_timeout(HTTP_TIMEOUT_SECS)
        .send()
        .map_err(|e| format!("HTTP GET to {} failed: {}", url, e))?;

    let status = response.status_code as u16;
    let body = response
        .as_str()
        .map(|s| s.to_string())
        .map_err(|e| format!("Failed to read response body: {}", e))?;

    Ok((status, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_encode_simple() {
        assert_eq!(url_encode("hello"), "hello");
        assert_eq!(url_encode("hello world"), "hello%20world");
        assert_eq!(url_encode("foo@bar.com"), "foo%40bar.com");
    }

    #[test]
    fn test_url_encode_unreserved() {
        // Unreserved chars should pass through
        assert_eq!(url_encode("A-Z_a-z.0~9"), "A-Z_a-z.0~9");
    }

    #[test]
    fn test_build_url() {
        let url = build_url("https://example.com/auth", &[
            ("client_id", "abc"),
            ("scope", "openid email"),
        ]);
        assert_eq!(
            url,
            "https://example.com/auth?client_id=abc&scope=openid%20email"
        );
    }

    #[test]
    fn test_build_url_no_params() {
        let url = build_url("https://example.com/auth", &[]);
        assert_eq!(url, "https://example.com/auth");
    }
}
