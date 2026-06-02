//! SSL certificate installation (InstallSslCert — local disk write + vhost patch).
//!
//! Note: The proto does not define an InstallSslCert RPC, but this module is
//! called internally by other subsystems (e.g. ACME renewal daemon orbit-cron)
//! and provides the helper functions used when cert material arrives.

use crate::validation::{unix_user_for_domain, validate_domain};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use tonic::Status;
use tracing::info;

/// Install a TLS certificate pair for a domain.
///
/// Writes cert.pem (0644) and key.pem (0600) into /etc/orbitcp/ssl/{domain}/,
/// patches the nginx vhost to enable HTTPS, and reloads nginx.
///
/// Returns the certificate expiry date as an RFC 3339 string.
pub fn install_ssl_cert(
    domain: &str,
    cert_pem: &str,
    key_pem: &str,
) -> Result<String, Status> {
    let domain = validate_domain(domain)?;

    // 1. Write cert files ───────────────────────────────────────────────────
    let ssl_dir = format!("/etc/orbitcp/ssl/{}", domain);
    std::fs::create_dir_all(&ssl_dir)
        .map_err(|e| Status::internal(format!("create SSL dir failed: {}", e)))?;

    let cert_path = format!("{}/cert.pem", ssl_dir);
    let key_path  = format!("{}/key.pem", ssl_dir);

    std::fs::write(&cert_path, cert_pem)
        .map_err(|e| Status::internal(format!("write cert.pem failed: {}", e)))?;
    std::fs::set_permissions(&cert_path, std::fs::Permissions::from_mode(0o644))
        .map_err(|e| Status::internal(format!("chmod cert.pem failed: {}", e)))?;

    std::fs::write(&key_path, key_pem)
        .map_err(|e| Status::internal(format!("write key.pem failed: {}", e)))?;
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| Status::internal(format!("chmod key.pem failed: {}", e)))?;

    info!(%domain, cert_path = %cert_path, key_path = %key_path, "SSL cert installed");

    // 2. Patch nginx vhost to add SSL server block ──────────────────────────
    // Derive the unix_user so the root directive in the SSL block is correct.
    let unix_user = unix_user_for_domain(&domain);
    super::webserver::append_ssl_to_nginx_vhost(&domain, &unix_user, &cert_path, &key_path)?;

    // 3. Parse expiry date from cert ───────────────────────────────────────
    let expiry = parse_cert_expiry(&cert_path).unwrap_or_else(|| "unknown".to_string());

    info!(%domain, expiry = %expiry, "SSL cert activated");
    Ok(expiry)
}

/// Parse the certificate expiry date using `openssl x509 -enddate -noout`.
/// Returns an ISO 8601 / RFC 3339 date string, or None on failure.
fn parse_cert_expiry(cert_path: &str) -> Option<String> {
    let output = Command::new("/usr/bin/openssl")
        .args(["x509", "-enddate", "-noout", "-in", cert_path])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    // Output: "notAfter=Mar 25 12:00:00 2026 GMT\n"
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line   = stdout.trim();

    let date_str = line.strip_prefix("notAfter=")?.trim();

    // Parse with chrono: "Mar 25 12:00:00 2026 GMT"
    // openssl outputs in "%b %e %H:%M:%S %Y GMT" format
    use chrono::DateTime;
    if let Ok(dt) = DateTime::parse_from_str(&format!("{} +0000", date_str), "%b %e %H:%M:%S %Y GMT %z") {
        return Some(dt.to_rfc3339());
    }

    // Fallback: return the raw string
    Some(date_str.to_string())
}

/// Remove SSL cert files for a domain (called during site deletion).
pub fn remove_ssl_cert(domain: &str) -> Result<(), Status> {
    let domain  = validate_domain(domain)?;
    let ssl_dir = format!("/etc/orbitcp/ssl/{}", domain);

    if Path::new(&ssl_dir).exists() {
        std::fs::remove_dir_all(&ssl_dir)
            .map_err(|e| Status::internal(format!("remove SSL dir failed: {}", e)))?;
        info!(%domain, "SSL cert removed");
    }
    Ok(())
}
