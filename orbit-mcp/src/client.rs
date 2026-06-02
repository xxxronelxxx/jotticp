use anyhow::Result;
use reqwest::{Client, StatusCode};
use serde_json::Value;
use std::time::Duration;

/// HTTP client that wraps all orbit-panel REST API calls.
/// Authenticates with a long-lived machine JWT stored in `ORBIT_API_KEY`.
pub struct OrbitClient {
    http:     Client,
    base_url: String,
    api_key:  String,
}

impl OrbitClient {
    /// Build from environment variables:
    /// - `ORBIT_PANEL_URL` (default: http://127.0.0.1:2087)
    /// - `ORBIT_API_KEY`   (required)
    pub fn from_env() -> Result<Self> {
        let base_url = std::env::var("ORBIT_PANEL_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:2087".into());
        let api_key = std::env::var("ORBIT_API_KEY")
            .map_err(|_| anyhow::anyhow!("ORBIT_API_KEY environment variable not set"))?;

        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("orbit-mcp/0.1.0")
            .build()?;

        Ok(Self { http, base_url, api_key })
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    async fn get(&self, path: &str) -> Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self.http
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await?;
        self.handle_response(resp).await
    }

    async fn post(&self, path: &str, body: &Value) -> Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self.http
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(body)
            .send()
            .await?;
        self.handle_response(resp).await
    }

    async fn delete(&self, path: &str) -> Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self.http
            .delete(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await?;
        self.handle_response(resp).await
    }

    async fn handle_response(&self, resp: reqwest::Response) -> Result<Value> {
        let status = resp.status();
        let body = resp.text().await?;

        if status == StatusCode::NO_CONTENT || body.is_empty() {
            return Ok(serde_json::json!({"success": true, "status": status.as_u16()}));
        }

        let json: Value = serde_json::from_str(&body).unwrap_or_else(|_| {
            serde_json::json!({"raw": body, "status": status.as_u16()})
        });

        if !status.is_success() {
            anyhow::bail!(
                "orbit-panel API error {}: {}",
                status,
                json.get("message")
                    .or_else(|| json.get("error"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error")
            );
        }

        Ok(json)
    }

    // ── Sites ─────────────────────────────────────────────────────────────────

    pub async fn list_sites(&self, search: Option<&str>) -> Result<Value> {
        let path = match search {
            Some(q) => format!("/api/v1/sites?search={}", urlencoding(q)),
            None    => "/api/v1/sites".into(),
        };
        self.get(&path).await
    }

    pub async fn create_site(&self, body: &Value) -> Result<Value> {
        self.post("/api/v1/sites", body).await
    }

    pub async fn delete_site(&self, site_id: &str) -> Result<Value> {
        self.delete(&format!("/api/v1/sites/{}", site_id)).await
    }

    pub async fn get_site_stats(&self, site_id: &str) -> Result<Value> {
        self.get(&format!("/api/v1/sites/{}/stats", site_id)).await
    }

    // ── Email ─────────────────────────────────────────────────────────────────

    pub async fn list_email_accounts(&self, domain: Option<&str>) -> Result<Value> {
        let path = match domain {
            Some(d) => format!("/api/v1/email?domain={}", urlencoding(d)),
            None    => "/api/v1/email".into(),
        };
        self.get(&path).await
    }

    pub async fn create_email_account(&self, body: &Value) -> Result<Value> {
        self.post("/api/v1/email/accounts", body).await
    }

    pub async fn delete_email_account(&self, account_id: &str) -> Result<Value> {
        self.delete(&format!("/api/v1/email/accounts/{}", account_id)).await
    }

    // ── DNS ───────────────────────────────────────────────────────────────────

    pub async fn list_dns_zones(&self, search: Option<&str>) -> Result<Value> {
        let path = match search {
            Some(q) => format!("/api/v1/dns/zones?search={}", urlencoding(q)),
            None    => "/api/v1/dns/zones".into(),
        };
        self.get(&path).await
    }

    pub async fn create_dns_record(&self, zone_id: &str, body: &Value) -> Result<Value> {
        self.post(&format!("/api/v1/dns/zones/{}/records", zone_id), body).await
    }

    // ── SSL ───────────────────────────────────────────────────────────────────

    pub async fn list_ssl_certs(&self, expiring_soon: bool) -> Result<Value> {
        let path = if expiring_soon {
            "/api/v1/ssl?expiring_soon=true"
        } else {
            "/api/v1/ssl"
        };
        self.get(path).await
    }

    pub async fn renew_ssl(&self, cert_id: &str) -> Result<Value> {
        self.post(&format!("/api/v1/ssl/{}/renew", cert_id), &serde_json::json!({})).await
    }

    // ── Databases ─────────────────────────────────────────────────────────────

    pub async fn list_databases(&self, site_id: Option<&str>) -> Result<Value> {
        let path = match site_id {
            Some(id) => format!("/api/v1/databases?site_id={}", id),
            None     => "/api/v1/databases".into(),
        };
        self.get(&path).await
    }

    pub async fn create_database(&self, body: &Value) -> Result<Value> {
        self.post("/api/v1/databases", body).await
    }

    // ── Backups ───────────────────────────────────────────────────────────────

    pub async fn trigger_backup(&self, site_id: &str, body: &Value) -> Result<Value> {
        self.post(&format!("/api/v1/backups/sites/{}/backup", site_id), body).await
    }

    // ── Servers ───────────────────────────────────────────────────────────────

    pub async fn get_server_stats(&self, server_id: Option<&str>) -> Result<Value> {
        let path = match server_id {
            Some(id) => format!("/api/v1/servers/{}", id),
            None     => "/api/v1/servers".into(),
        };
        self.get(&path).await
    }

    // ── Apps ──────────────────────────────────────────────────────────────────

    pub async fn install_app(&self, body: &Value) -> Result<Value> {
        self.post("/api/v1/apps/install", body).await
    }

    pub async fn list_installed_apps(&self, site_id: Option<&str>) -> Result<Value> {
        let path = match site_id {
            Some(id) => format!("/api/v1/apps/installed?site_id={}", id),
            None     => "/api/v1/apps/installed".into(),
        };
        self.get(&path).await
    }

    // ── Cron ──────────────────────────────────────────────────────────────────

    pub async fn list_cron_jobs(&self, site_id: Option<&str>) -> Result<Value> {
        let path = match site_id {
            Some(id) => format!("/api/v1/cron?site_id={}", id),
            None     => "/api/v1/cron".into(),
        };
        self.get(&path).await
    }

    pub async fn create_cron_job(&self, body: &Value) -> Result<Value> {
        self.post("/api/v1/cron", body).await
    }

    // ── Audit log ─────────────────────────────────────────────────────────────

    pub async fn get_system_logs(&self, limit: Option<u32>, action: Option<&str>, user_id: Option<&str>) -> Result<Value> {
        let mut params = vec![];
        if let Some(l) = limit {
            params.push(format!("limit={}", l.min(200)));
        }
        if let Some(a) = action {
            params.push(format!("action={}", urlencoding(a)));
        }
        if let Some(u) = user_id {
            params.push(format!("user_id={}", u));
        }
        let query = if params.is_empty() {
            String::new()
        } else {
            format!("?{}", params.join("&"))
        };
        self.get(&format!("/api/v1/audit-log{}", query)).await
    }
}

/// Minimal percent-encoding for query string values.
fn urlencoding(s: &str) -> String {
    s.chars().fold(String::new(), |mut out, c| {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(c),
            _ => {
                for byte in c.to_string().as_bytes() {
                    out.push_str(&format!("%{:02X}", byte));
                }
            }
        }
        out
    })
}
