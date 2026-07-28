use std::sync::Arc;
use crate::AppState;

// ── Email service helpers ─────────────────────────────────────────────────────
// These functions are called by jotti-panel API handlers and background jobs.
// The actual email server management (Postfix/Dovecot config writes) is done
// by jotti-mail daemon at 127.0.0.1:5354. This module provides the HTTP
// client wrapper.

const ORBIT_MAIL_URL: &str = "http://127.0.0.1:5354";

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct MailDomainStatus {
    pub domain:        String,
    pub active:        bool,
    pub dkim_enabled:  bool,
    pub dkim_selector: Option<String>,
    pub spf_record:    Option<String>,
}

/// Create a mail domain in jotti-mail (Postfix virtual_domains + DKIM key setup).
pub async fn create_mail_domain(domain: &str) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/domains", ORBIT_MAIL_URL))
        .json(&serde_json::json!({ "domain": domain }))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("jotti-mail unreachable: {}", e))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("jotti-mail error creating domain {}: {}", domain, body));
    }

    tracing::info!(%domain, "Mail domain created via jotti-mail");
    Ok(())
}

/// Delete a mail domain (Postfix virtual_domains, all accounts, DKIM keys).
pub async fn delete_mail_domain(domain: &str) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let resp = client
        .delete(format!("{}/api/domains/{}", ORBIT_MAIL_URL, domain))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("jotti-mail unreachable: {}", e))?;

    // 404 is acceptable (already deleted)
    if !resp.status().is_success() && resp.status() != reqwest::StatusCode::NOT_FOUND {
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("jotti-mail error deleting domain {}: {}", domain, body));
    }

    tracing::info!(%domain, "Mail domain deleted via jotti-mail");
    Ok(())
}

/// Create a mailbox account (Postfix virtual_mailbox + Dovecot password file entry).
pub async fn create_mailbox(domain: &str, username: &str, password: &str, quota_mb: u32) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/domains/{}/accounts", ORBIT_MAIL_URL, domain))
        .json(&serde_json::json!({
            "username": username,
            "password": password,
            "quota_mb": quota_mb,
        }))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("jotti-mail unreachable: {}", e))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("jotti-mail error creating mailbox {}@{}: {}", username, domain, body));
    }

    tracing::info!(%username, %domain, "Mailbox created via jotti-mail");
    Ok(())
}

/// Delete a mailbox (Postfix virtual_mailbox + Dovecot password file + spool).
pub async fn delete_mailbox(domain: &str, username: &str) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let resp = client
        .delete(format!("{}/api/domains/{}/accounts/{}", ORBIT_MAIL_URL, domain, username))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("jotti-mail unreachable: {}", e))?;

    if !resp.status().is_success() && resp.status() != reqwest::StatusCode::NOT_FOUND {
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("jotti-mail error deleting mailbox {}@{}: {}", username, domain, body));
    }

    tracing::info!(%username, %domain, "Mailbox deleted via jotti-mail");
    Ok(())
}

/// Enable DKIM for a domain (rspamadm dkim_keygen + DNS record insertion).
pub async fn enable_dkim(domain: &str) -> anyhow::Result<String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/domains/{}/dkim", ORBIT_MAIL_URL, domain))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("jotti-mail unreachable: {}", e))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("jotti-mail error enabling DKIM for {}: {}", domain, body));
    }

    let body: serde_json::Value = resp.json().await
        .map_err(|e| anyhow::anyhow!("Invalid jotti-mail response: {}", e))?;

    let dns_value = body["dns_txt_value"]
        .as_str()
        .unwrap_or("")
        .to_string();

    tracing::info!(%domain, "DKIM enabled via jotti-mail");
    Ok(dns_value)
}

/// Check SMTP connectivity (used by settings SMTP test endpoint).
pub async fn test_smtp_connectivity(host: &str, port: u16, timeout_secs: u64) -> anyhow::Result<()> {
    let addr = format!("{}:{}", host, port);
    tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        tokio::net::TcpStream::connect(&addr),
    )
    .await
    .map_err(|_| anyhow::anyhow!("Connection timed out after {}s", timeout_secs))?
    .map_err(|e| anyhow::anyhow!("Cannot connect to {}:{} — {}", host, port, e))?;

    tracing::info!(%host, %port, "SMTP connectivity test passed");
    Ok(())
}

/// Check if jotti-mail is healthy.
pub async fn health_check() -> bool {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap_or_default();

    client
        .get(format!("{}/health", ORBIT_MAIL_URL))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}
