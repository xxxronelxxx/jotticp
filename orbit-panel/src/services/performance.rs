/// Benchmark utilities and HTTP/3 detection for OrbitCP.
///
/// Used by GET /api/v1/settings/performance to surface:
/// - Whether the running nginx binary supports QUIC/HTTP3
/// - Estimated TTFB against the local panel
/// - Count of active sites
/// - Nginx version string
/// - Actionable performance recommendations
use std::sync::Arc;

use crate::AppState;

// ── Public types ──────────────────────────────────────────────────────────────

pub struct PerfReport {
    pub http3_supported: bool,
    pub ttfb_ms:         f64,
    pub active_sites:    u64,
    pub nginx_version:   String,
    pub cache_hit_rate:  f64,
    pub recommendations: Vec<String>,
}

// ── HTTP/3 detection ──────────────────────────────────────────────────────────

/// Check if the system nginx binary was compiled with HTTP/3 (QUIC) support.
/// Runs `nginx -V` and inspects stderr for the quic module flag.
pub async fn check_http3_support() -> bool {
    let output = tokio::process::Command::new("nginx")
        .arg("-V")
        .output()
        .await;

    if let Ok(out) = output {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return stderr.contains("--with-http_v3_module") || stderr.contains("quic");
    }
    false
}

/// Extract the nginx version string from `nginx -V` output.
pub async fn get_nginx_version() -> String {
    let output = tokio::process::Command::new("nginx")
        .arg("-V")
        .output()
        .await;

    if let Ok(out) = output {
        let stderr = String::from_utf8_lossy(&out.stderr);
        // e.g. "nginx version: nginx/1.25.3"
        if let Some(line) = stderr.lines().next() {
            if let Some(ver) = line.strip_prefix("nginx version: nginx/") {
                return ver.trim().to_string();
            }
        }
    }
    "unknown".to_string()
}

// ── TTFB estimate ─────────────────────────────────────────────────────────────

/// Make a local GET /health request and measure elapsed wall time as a TTFB proxy.
/// Returns milliseconds (f64).
pub async fn get_ttfb_estimate() -> f64 {
    let start = std::time::Instant::now();
    let _ = reqwest::get("http://127.0.0.1:2087/health").await;
    start.elapsed().as_secs_f64() * 1000.0
}

// ── Full report ───────────────────────────────────────────────────────────────

pub async fn get_performance_report(state: &Arc<AppState>) -> PerfReport {
    let (http3_supported, ttfb_ms, nginx_version) = tokio::join!(
        check_http3_support(),
        get_ttfb_estimate(),
        get_nginx_version(),
    );

    let active_sites: u64 = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM sites WHERE status = 'active' AND deleted_at IS NULL"
    )
    .fetch_one(&state.db)
    .await
    .unwrap_or(Some(0))
    .unwrap_or(0) as u64;

    // Build actionable recommendations
    let mut recommendations: Vec<String> = Vec::new();

    if !http3_supported {
        recommendations.push(
            "Enable HTTP/3 in nginx by recompiling with --with-http_v3_module (nginx >= 1.25.0)".to_string(),
        );
    }

    if ttfb_ms > 50.0 {
        recommendations.push(format!(
            "TTFB is {ttfb_ms:.1}ms — consider enabling connection pooling or reducing middleware overhead"
        ));
    }

    // Check for Brotli support
    let brotli_check = tokio::process::Command::new("nginx")
        .arg("-V")
        .output()
        .await;
    let has_brotli = brotli_check
        .map(|o| String::from_utf8_lossy(&o.stderr).contains("brotli"))
        .unwrap_or(false);
    if !has_brotli {
        recommendations.push(
            "Consider enabling Brotli compression (ngx_brotli module) for 15-25% smaller responses".to_string(),
        );
    }

    if recommendations.is_empty() {
        recommendations.push("Your configuration looks good! Monitor TTFB under load.".to_string());
    }

    PerfReport {
        http3_supported,
        ttfb_ms,
        active_sites,
        nginx_version,
        cache_hit_rate: 0.0, // TODO: read from OPcache stats once php_fpm_stats integrated
        recommendations,
    }
}
