//! HTTP metrics middleware — records counters and histograms exported at /metrics.
//!
//! Metric naming follows Prometheus best practice:
//!   - http_requests_total{method,status}: total request count
//!   - http_request_duration_seconds{method,path}: request latency histogram
//!
//! `path` is intentionally not used in the counter label to avoid high-cardinality
//! explosion (one series per UUID would be catastrophic).

use axum::{extract::Request, middleware::Next, response::Response};
use std::time::Instant;

pub async fn track(req: Request, next: Next) -> Response {
    let start = Instant::now();
    let method = req.method().clone();

    // Normalize the path: keep only the first two path segments to bound cardinality.
    // /api/v1/sites/abc-123/stats -> /api/v1/sites
    let raw_path = req.uri().path().to_string();
    let route_label = bucket_path(&raw_path);

    let response = next.run(req).await;
    let elapsed = start.elapsed().as_secs_f64();
    let status = response.status().as_u16();

    metrics::counter!(
        "http_requests_total",
        "method" => method.to_string(),
        "status" => status.to_string(),
        "route"  => route_label.clone(),
    ).increment(1);

    metrics::histogram!(
        "http_request_duration_seconds",
        "method" => method.to_string(),
        "route"  => route_label,
    ).record(elapsed);

    response
}

/// Reduce path to first two segments to bound metric cardinality.
fn bucket_path(p: &str) -> String {
    let parts: Vec<&str> = p.split('/').filter(|s| !s.is_empty()).take(3).collect();
    if parts.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", parts.join("/"))
    }
}
