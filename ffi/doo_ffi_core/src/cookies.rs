//! HTTP Cookie Management — Core Infrastructure
//!
//! Centralized cookie handling used by ALL auth strategies (JWT, OAuth, future).
//! This is a CORE auth concept — not specific to any one strategy.
//!
//! ## Architecture
//!
//! Auth crates push cookies via `set_response_cookie()`.
//! HTTP server takes them via `take_response_cookies()` and adds Set-Cookie headers.
//! This decouples auth from HTTP — auth sets intent, HTTP executes it.
//!
//! ## Security Defaults (industry best practice)
//!
//! - HttpOnly: true (prevents XSS token theft via JavaScript)
//! - Secure: true in production (HTTPS only)
//! - SameSite: Lax for access token (allows navigation), Strict for refresh
//! - Path scoping: refresh token only sent to /auth/refresh

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

// ============================================================================
// RUNTIME COOKIE SECURITY OVERRIDE
// ============================================================================

/// Set by doo_ffi_http during CORS freeze when non-HTTPS origins are detected.
/// When true, cookies will NOT have the Secure flag (allows HTTP in dev).
/// This provides a runtime fallback independent of DOO_DEV env var propagation.
static INSECURE_COOKIES: AtomicBool = AtomicBool::new(false);
static INSECURE_COOKIES_SET: AtomicBool = AtomicBool::new(false);

/// Called by doo_ffi_http during CORS freeze to inform the cookie system
/// whether cookies should omit the Secure flag (e.g., when CORS origins use http://).
///
/// This is a runtime signal that doesn't depend on env var propagation.
pub fn set_insecure_cookies(insecure: bool) {
    INSECURE_COOKIES.store(insecure, Ordering::Relaxed);
    INSECURE_COOKIES_SET.store(true, Ordering::Relaxed);
}

// ============================================================================
// RUNTIME COOKIE DOMAIN — Cross-subdomain cookie sharing
// ============================================================================

/// Cookie domain derived from CORS origins at startup.
/// When set, cookies include `Domain=.parent.tld` so they're shared across subdomains.
/// Example: CORS origin `https://app.example.com` → cookie domain `.example.com`
///
/// This enables the standard cross-subdomain architecture where the API and frontend
/// live on different subdomains (e.g., `api.example.com` + `app.example.com`).
static COOKIE_DOMAIN: OnceLock<Option<String>> = OnceLock::new();

/// Called by doo_ffi_http during CORS freeze to set the cookie domain.
/// Extracts the parent domain from CORS origins for cross-subdomain cookie sharing.
///
/// Only sets the domain when:
/// - CORS credentials are enabled (cookies are being shared cross-origin)
/// - CORS origin is a specific domain (not "*")
/// - Origin has a subdomain (e.g., `app.example.com`, not `example.com`)
pub fn set_cookie_domain(domain: Option<String>) {
    let _ = COOKIE_DOMAIN.set(domain);
}

/// Get the configured cookie domain (if any).
pub fn get_cookie_domain() -> Option<&'static str> {
    COOKIE_DOMAIN.get().and_then(|opt| opt.as_deref())
}

// ============================================================================
// COOKIE NAMES — Single Source of Truth
// ============================================================================

/// Cookie name for the access token
pub const COOKIE_ACCESS_TOKEN: &str = "doo_access_token";

/// Cookie name for the refresh token
pub const COOKIE_REFRESH_TOKEN: &str = "doo_refresh_token";

// ============================================================================
// COOKIE BUILDER — Constructs Set-Cookie header values
// ============================================================================

/// A cookie to be set in the HTTP response.
#[derive(Debug, Clone)]
pub struct ResponseCookie {
    pub name: String,
    pub value: String,
    pub max_age: Option<i64>,
    pub path: Option<String>,
    pub domain: Option<String>,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: SameSite,
}

/// SameSite attribute for cookies
#[derive(Debug, Clone, Copy)]
pub enum SameSite {
    /// Cookie sent on same-site requests and top-level navigations (recommended for access token)
    Lax,
    /// Cookie only sent on same-site requests (recommended for refresh token)
    Strict,
    /// Cookie sent on all requests (not recommended for auth)
    None,
}

impl ResponseCookie {
    /// Create a new cookie with secure defaults.
    pub fn new(name: &str, value: &str) -> Self {
        Self {
            name: name.to_string(),
            value: value.to_string(),
            max_age: None,
            path: Some("/".to_string()),
            domain: None,
            secure: should_secure_cookie(),
            http_only: true,
            same_site: SameSite::Lax,
        }
    }

    /// Create an access token cookie with recommended settings.
    pub fn access_token(token: &str, max_age_secs: i64) -> Self {
        Self {
            name: COOKIE_ACCESS_TOKEN.to_string(),
            value: token.to_string(),
            max_age: Some(max_age_secs),
            path: Some("/".to_string()),
            domain: None,
            secure: should_secure_cookie(),
            http_only: true,
            same_site: SameSite::Lax, // allows navigation (OAuth redirects)
        }
    }

    /// Create a refresh token cookie with restricted settings.
    pub fn refresh_token(token: &str, max_age_secs: i64, refresh_path: &str) -> Self {
        Self {
            name: COOKIE_REFRESH_TOKEN.to_string(),
            value: token.to_string(),
            max_age: Some(max_age_secs),
            path: Some(refresh_path.to_string()),
            domain: None,
            secure: should_secure_cookie(),
            http_only: true,
            same_site: SameSite::Strict, // only same-site (no cross-site refresh)
        }
    }

    /// Create a cookie that clears/deletes an existing cookie.
    pub fn clear(name: &str, path: &str) -> Self {
        Self {
            name: name.to_string(),
            value: String::new(),
            max_age: Some(0),
            path: Some(path.to_string()),
            domain: None,
            secure: should_secure_cookie(),
            http_only: true,
            same_site: SameSite::Lax,
        }
    }

    pub fn with_domain(mut self, domain: &str) -> Self {
        self.domain = Some(domain.to_string());
        self
    }

    pub fn with_same_site(mut self, same_site: SameSite) -> Self {
        self.same_site = same_site;
        self
    }

    /// Serialize to Set-Cookie header value.
    pub fn to_header_value(&self) -> String {
        let mut parts = vec![format!("{}={}", self.name, self.value)];

        if let Some(max_age) = self.max_age {
            parts.push(format!("Max-Age={}", max_age));
        }

        if let Some(ref path) = self.path {
            parts.push(format!("Path={}", path));
        }

        if let Some(ref domain) = self.domain {
            parts.push(format!("Domain={}", domain));
        }

        if self.secure {
            parts.push("Secure".to_string());
        }

        if self.http_only {
            parts.push("HttpOnly".to_string());
        }

        match self.same_site {
            SameSite::Lax => parts.push("SameSite=Lax".to_string()),
            SameSite::Strict => parts.push("SameSite=Strict".to_string()),
            SameSite::None => parts.push("SameSite=None".to_string()),
        }

        parts.join("; ")
    }
}

// ============================================================================
// THREAD-LOCAL PENDING COOKIES — Auth sets, HTTP server reads
// ============================================================================

thread_local! {
    static PENDING_COOKIES: RefCell<Vec<ResponseCookie>> = RefCell::new(Vec::new());
}

/// Push a cookie to be set in the next HTTP response.
///
/// Called by auth strategies (OAuth callback, login handler, refresh endpoint).
/// The HTTP server calls `take_response_cookies()` to retrieve and clear them.
pub fn set_response_cookie(cookie: ResponseCookie) {
    PENDING_COOKIES.with(|c| c.borrow_mut().push(cookie));
}

/// Take all pending cookies and clear the list.
///
/// Called by the HTTP server after building the response, to add Set-Cookie headers.
/// Returns an empty Vec if no cookies were set.
pub fn take_response_cookies() -> Vec<ResponseCookie> {
    PENDING_COOKIES.with(|c| {
        let mut cookies = c.borrow_mut();
        std::mem::take(&mut *cookies)
    })
}

/// Clear pending cookies without returning them.
///
/// Called on error paths or request cleanup to prevent cookie leaks.
pub fn clear_pending_cookies() {
    PENDING_COOKIES.with(|c| c.borrow_mut().clear());
}

/// Push a pre-built Set-Cookie header value into the pending cookies.
///
/// This is used by the cross-DLL cookie bridge: auth DLL builds the cookie header
/// string, passes it via FFI to the HTTP DLL, which calls this function to store
/// it in the HTTP DLL's thread-local where the server can find it.
///
/// The header value is stored as-is in a RawCookie. The server writes it directly
/// as a Set-Cookie response header without reparsing.
pub fn push_raw_cookie_header(header_value: String) {
    PENDING_RAW_COOKIES.with(|c| c.borrow_mut().push(header_value));
}

/// Take all pending raw cookie headers and clear the list.
pub fn take_raw_cookies() -> Vec<String> {
    PENDING_RAW_COOKIES.with(|c| {
        let mut cookies = c.borrow_mut();
        std::mem::take(&mut *cookies)
    })
}

/// Thread-local raw cookie headers pushed via cross-DLL bridge.
thread_local! {
    static PENDING_RAW_COOKIES: RefCell<Vec<String>> = RefCell::new(Vec::new());
}

// ============================================================================
// COOKIE PARSING — Extract token from Cookie header
// ============================================================================

/// Extract a specific cookie value from the Cookie header string.
///
/// Cookie header format: "name1=value1; name2=value2; name3=value3"
/// Returns None if the cookie is not found.
pub fn extract_cookie_value<'a>(cookie_header: &'a str, cookie_name: &str) -> Option<&'a str> {
    for part in cookie_header.split(';') {
        let trimmed = part.trim();
        if let Some(value) = trimmed.strip_prefix(cookie_name) {
            if let Some(value) = value.strip_prefix('=') {
                return Some(value);
            }
        }
    }
    None
}

// ============================================================================
// HELPERS
// ============================================================================

/// Determine whether cookie Secure flag should be set.
///
/// Returns false (no Secure flag) when ANY of these is true:
/// - Debug build (`cfg!(debug_assertions)`)
/// - DOO_DEV=1 or DOO_DEV=true env var
/// - Runtime flag set by doo_ffi_http when CORS origins use http://
///
/// Returns true only in production (release build, no DOO_DEV, HTTPS origins).
fn should_secure_cookie() -> bool {
    !is_dev_mode()
}

fn is_dev_mode() -> bool {
    cfg!(debug_assertions)
        || std::env::var(crate::constants::ENV_DOO_DEV)
            .map(|v| v == "1" || v == "true")
            .unwrap_or(false)
        || (INSECURE_COOKIES_SET.load(Ordering::Relaxed)
            && INSECURE_COOKIES.load(Ordering::Relaxed))
}

/// Get the auth base path from env (single source of truth).
/// Used for cookie path scoping on refresh tokens.
fn get_auth_base_path() -> String {
    std::env::var(crate::constants::ENV_AUTH_BASE_PATH).unwrap_or_else(|_| "/auth".to_string())
}

/// Get the refresh endpoint path (base_path + "/refresh").
fn get_refresh_path() -> String {
    get_auth_base_path() + "/refresh"
}

// ============================================================================
// HIGH-LEVEL AUTH COOKIE API — Single source of truth for ALL auth strategies
// ============================================================================

/// Push auth cookies (access + refresh) onto the pending response.
///
/// **This is the ONE function all auth code calls** — OAuth callback, refresh
/// endpoint, `Auth.login()`, `Auth.refresh()`. No caller builds cookies manually.
///
/// Automatically applies the cookie domain (if configured via CORS) for
/// cross-subdomain cookie sharing. This enables architectures where the API
/// and frontend live on different subdomains (e.g., `api.x.com` + `app.x.com`).
///
/// - access_token: The access JWT
/// - refresh_token: Optional refresh JWT (None = only set access cookie)
/// - access_expiry_secs: Access token lifetime in seconds
/// - refresh_expiry_secs: Refresh token lifetime in seconds
pub fn push_auth_cookies(
    access_token: &str,
    refresh_token: Option<&str>,
    access_expiry_secs: i64,
    refresh_expiry_secs: i64,
) {
    let mut access = ResponseCookie::access_token(access_token, access_expiry_secs);
    if let Some(domain) = get_cookie_domain() {
        access = access.with_domain(domain);
    }
    set_response_cookie(access);

    if let Some(refresh) = refresh_token {
        let mut refresh_cookie =
            ResponseCookie::refresh_token(refresh, refresh_expiry_secs, &get_refresh_path());
        if let Some(domain) = get_cookie_domain() {
            refresh_cookie = refresh_cookie.with_domain(domain);
        }
        set_response_cookie(refresh_cookie);
    }
}

/// Push clear-cookies onto the pending response — logs the user out.
///
/// Sets Max-Age=0 on both access and refresh cookies, causing the browser
/// to delete them. **This is the ONE logout cookie function.**
/// Applies the same cookie domain as push_auth_cookies so the browser
/// correctly matches and deletes the cookies.
pub fn push_clear_cookies() {
    let mut access = ResponseCookie::clear(COOKIE_ACCESS_TOKEN, "/");
    if let Some(domain) = get_cookie_domain() {
        access = access.with_domain(domain);
    }
    set_response_cookie(access);

    let mut refresh = ResponseCookie::clear(COOKIE_REFRESH_TOKEN, &get_refresh_path());
    if let Some(domain) = get_cookie_domain() {
        refresh = refresh.with_domain(domain);
    }
    set_response_cookie(refresh);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cookie_serialization() {
        let cookie = ResponseCookie::access_token("eyJ-test", 3600);
        let header = cookie.to_header_value();
        assert!(header.contains("doo_access_token=eyJ-test"));
        assert!(header.contains("Max-Age=3600"));
        assert!(header.contains("Path=/"));
        assert!(header.contains("HttpOnly"));
        assert!(header.contains("SameSite=Lax"));
    }

    #[test]
    fn test_refresh_cookie_strict() {
        let cookie = ResponseCookie::refresh_token("eyJ-refresh", 604800, "/auth/refresh");
        let header = cookie.to_header_value();
        assert!(header.contains("doo_refresh_token=eyJ-refresh"));
        assert!(header.contains("Path=/auth/refresh"));
        assert!(header.contains("SameSite=Strict"));
    }

    #[test]
    fn test_clear_cookie() {
        let cookie = ResponseCookie::clear("doo_access_token", "/");
        let header = cookie.to_header_value();
        assert!(header.contains("Max-Age=0"));
    }

    #[test]
    fn test_extract_cookie_value() {
        let header = "doo_access_token=abc123; doo_refresh_token=xyz789; other=val";
        assert_eq!(
            extract_cookie_value(header, "doo_access_token"),
            Some("abc123")
        );
        assert_eq!(
            extract_cookie_value(header, "doo_refresh_token"),
            Some("xyz789")
        );
        assert_eq!(extract_cookie_value(header, "nonexistent"), None);
    }

    #[test]
    fn test_pending_cookies() {
        clear_pending_cookies();
        set_response_cookie(ResponseCookie::access_token("token1", 3600));
        set_response_cookie(ResponseCookie::refresh_token(
            "token2",
            604800,
            "/auth/refresh",
        ));

        let cookies = take_response_cookies();
        assert_eq!(cookies.len(), 2);

        // Second take should be empty
        let cookies2 = take_response_cookies();
        assert_eq!(cookies2.len(), 0);
    }
}
