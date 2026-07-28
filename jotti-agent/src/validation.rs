//! Input validation helpers.
//!
//! All user-supplied strings that end up in shell commands or file paths MUST
//! pass through a validation function here before use.  No string is allowed
//! to reach std::process::Command arguments without explicit allowlisting.

use once_cell::sync::Lazy;
use regex::Regex;
use tonic::Status;

// ── Compiled patterns ─────────────────────────────────────────────────────────

/// Valid domain names: labels of [a-z0-9-], separated by dots.
/// Internationalised (punycode) domains are already in ASCII form at this layer.
static RE_DOMAIN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^([a-z0-9]([a-z0-9\-]{0,61}[a-z0-9])?\.)+[a-z]{2,}$").unwrap()
});

/// Valid PHP version strings: "8.1", "8.2", "8.3", "8.4" etc.
static RE_PHP_VERSION: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[0-9]\.[0-9]$").unwrap());

/// Valid Unix user names: lowercase, digits, underscores, max 32 chars.
static RE_UNIX_USER: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[a-z_][a-z0-9_]{0,31}$").unwrap());

/// Valid database names / db user names: [a-z0-9_], max 64 chars.
static RE_DB_IDENT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[a-z][a-z0-9_]{0,63}$").unwrap());

/// Valid absolute paths: must start with /, no .. components, no shell metacharacters.
static RE_SAFE_PATH: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(/[a-zA-Z0-9_.\-]+)+$").unwrap());

/// Valid DNS zone names (allow trailing dot).
static RE_DNS_ZONE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^([a-z0-9]([a-z0-9\-]{0,61}[a-z0-9])?\.)+[a-z]{2,}\.?$").unwrap()
});

// ── Validation functions ──────────────────────────────────────────────────────

/// Validate and normalise a domain name.  Returns lowercase domain on success.
pub fn validate_domain(domain: &str) -> Result<String, Status> {
    let lower = domain.to_lowercase();
    if lower.is_empty() || lower.len() > 253 {
        return Err(Status::invalid_argument("domain name must be 1–253 characters"));
    }
    if !RE_DOMAIN.is_match(&lower) {
        return Err(Status::invalid_argument(format!(
            "invalid domain name: {:?} — only [a-z0-9.-] allowed",
            domain
        )));
    }
    Ok(lower)
}

/// Validate a PHP version string.
pub fn validate_php_version(version: &str) -> Result<String, Status> {
    if !RE_PHP_VERSION.is_match(version) {
        return Err(Status::invalid_argument(format!(
            "invalid PHP version: {:?} — expected format like \"8.3\"",
            version
        )));
    }
    Ok(version.to_string())
}

/// Validate a Unix user name.
pub fn validate_unix_user(user: &str) -> Result<String, Status> {
    if !RE_UNIX_USER.is_match(user) {
        return Err(Status::invalid_argument(format!(
            "invalid Unix username: {:?}",
            user
        )));
    }
    Ok(user.to_string())
}

/// Validate a database name or database user name.
pub fn validate_db_ident(name: &str) -> Result<String, Status> {
    if !RE_DB_IDENT.is_match(name) {
        return Err(Status::invalid_argument(format!(
            "invalid database identifier: {:?} — only [a-z0-9_] allowed",
            name
        )));
    }
    Ok(name.to_string())
}

/// Validate an absolute filesystem path for use in commands.
/// Rejects `..`, shell metacharacters, and relative paths.
pub fn validate_safe_path(path: &str) -> Result<String, Status> {
    if path.contains("..") {
        return Err(Status::invalid_argument("path traversal (.. component) not allowed"));
    }
    if !path.starts_with('/') {
        return Err(Status::invalid_argument("path must be absolute"));
    }
    if !RE_SAFE_PATH.is_match(path) {
        return Err(Status::invalid_argument(format!(
            "unsafe path: {:?} — only [a-zA-Z0-9_.-/] allowed",
            path
        )));
    }
    Ok(path.to_string())
}

/// Validate a DNS zone name (with or without trailing dot).
pub fn validate_dns_zone(zone: &str) -> Result<String, Status> {
    let lower = zone.to_lowercase();
    if !RE_DNS_ZONE.is_match(&lower) {
        return Err(Status::invalid_argument(format!(
            "invalid DNS zone: {:?}",
            zone
        )));
    }
    Ok(lower)
}

/// Derive the canonical Unix user name from a validated domain.
/// "sub.example.com" → "orbit_sub_example_com" (capped at 32 chars total).
pub fn unix_user_for_domain(domain: &str) -> String {
    let sanitised: String = domain
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .take(24) // prefix "orbit_" adds 6 → total max 30
        .collect();
    format!("orbit_{}", sanitised)
}

/// Validate a web server identifier.
pub fn validate_web_server(ws: &str) -> Result<&str, Status> {
    match ws {
        "ols" | "nginx" | "apache" | "lse" | "" => Ok(ws),
        other => Err(Status::invalid_argument(format!(
            "unknown web_server: {:?} — expected ols | nginx | apache | lse",
            other
        ))),
    }
}

/// Validate a database type string.
pub fn validate_db_type(db_type: &str) -> Result<&str, Status> {
    match db_type {
        "mysql" | "mariadb" | "postgresql" | "ferretdb" | "surrealdb" => Ok(db_type),
        other => Err(Status::invalid_argument(format!(
            "unknown db_type: {:?}",
            other
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_domains() {
        assert!(validate_domain("example.com").is_ok());
        assert!(validate_domain("sub.example.co.uk").is_ok());
        assert!(validate_domain("my-site.example.org").is_ok());
    }

    #[test]
    fn reject_bad_domains() {
        assert!(validate_domain("").is_err());
        assert!(validate_domain("../etc/passwd").is_err());
        assert!(validate_domain("a b.com").is_err());
        assert!(validate_domain("bad;rm -rf").is_err());
    }

    #[test]
    fn unix_user_derivation() {
        assert_eq!(unix_user_for_domain("example.com"), "orbit_example_com");
        assert_eq!(unix_user_for_domain("sub.example.com"), "orbit_sub_example_com");
    }

    #[test]
    fn safe_path_rejects_traversal() {
        assert!(validate_safe_path("/home/orbit_example_com").is_ok());
        assert!(validate_safe_path("/home/../etc/passwd").is_err());
        assert!(validate_safe_path("relative/path").is_err());
    }
}
