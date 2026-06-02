use axum::{
    extract::Request,
    middleware::Next,
    response::Response,
    http::header,
};

/// Axum middleware function that injects security response headers on every response.
///
/// Headers added:
///   - Strict-Transport-Security: max-age=31536000; includeSubDomains; preload
///   - X-Content-Type-Options: nosniff
///   - X-Frame-Options: SAMEORIGIN
///   - Referrer-Policy: strict-origin
///   - Permissions-Policy: geolocation=(), microphone=(), camera=()
///   - Content-Security-Policy: strict; report-uri /api/v1/csp-report
///
/// Applied in main.rs as:
///   .layer(axum::middleware::from_fn(middleware::security_headers::security_headers))
pub async fn security_headers(req: Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    let h = response.headers_mut();

    // Prevent MIME type sniffing attacks
    h.insert(
        header::HeaderName::from_static("x-content-type-options"),
        header::HeaderValue::from_static("nosniff"),
    );

    // Anti-clickjacking — deny framing from other origins
    h.insert(
        header::HeaderName::from_static("x-frame-options"),
        header::HeaderValue::from_static("SAMEORIGIN"),
    );

    // Limit referrer leakage to origin-only on cross-origin requests
    h.insert(
        header::HeaderName::from_static("referrer-policy"),
        header::HeaderValue::from_static("strict-origin"),
    );

    // Disable browser features not used by the panel
    h.insert(
        header::HeaderName::from_static("permissions-policy"),
        header::HeaderValue::from_static("geolocation=(), microphone=(), camera=()"),
    );

    // HSTS: 1 year, all subdomains, preload-list eligible
    h.insert(
        header::STRICT_TRANSPORT_SECURITY,
        header::HeaderValue::from_static("max-age=31536000; includeSubDomains; preload"),
    );

    // CSP: strict inline-scripts blocked; 'unsafe-inline' for SvelteKit <style> tags only
    h.insert(
        header::HeaderName::from_static("content-security-policy"),
        header::HeaderValue::from_static(
            "default-src 'self'; \
             script-src 'self' https://static.cloudflareinsights.com; \
             style-src 'self' 'unsafe-inline'; \
             img-src 'self' data:; \
             font-src 'self'; \
             connect-src 'self' wss:; \
             frame-ancestors 'self'; \
             form-action 'self'; \
             base-uri 'self'; \
             report-uri /api/v1/csp-report"
        ),
    );

    response
}
