/// orbit — JottiCP command-line interface
///
/// Reads base URL from ~/.jottiecp/config.toml or ORBIT_URL env var.
/// Reads auth token from OS keychain (keyring) or ORBIT_TOKEN env var.
/// All list commands support --json for machine-readable output.

use std::{
    fs,
    path::PathBuf,
};

use anyhow::{bail, Context, Result};
use clap::{CommandFactory, Parser, Subcommand};
use colored::Colorize;
use keyring::Entry;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tabled::{Table, Tabled};

const KEYRING_SERVICE: &str = "jottiecp";
const KEYRING_USER: &str = "token";

// ─────────────────────────────────────────────────────────────────────────────
// Config file
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Default)]
struct CliConfig {
    url: Option<String>,
}

fn config_path() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".jottiecp").join("config.toml")
}

fn load_config() -> CliConfig {
    let path = config_path();
    match fs::read_to_string(&path) {
        Ok(content) => toml::from_str(&content).unwrap_or_default(),
        Err(_) => CliConfig::default(),
    }
}

fn save_config(cfg: &CliConfig) -> Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("create config dir")?;
    }
    let content = toml::to_string_pretty(cfg).context("serialize config")?;
    fs::write(&path, content).context("write config")?;
    Ok(())
}

fn get_base_url() -> Result<String> {
    if let Ok(url) = std::env::var("ORBIT_URL") {
        return Ok(url);
    }
    let cfg = load_config();
    cfg.url.context(
        "no base URL configured — run `orbit config set-url https://panel.example.com`",
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Token storage
// ─────────────────────────────────────────────────────────────────────────────

fn store_token(token: &str) -> Result<()> {
    let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER).context("create keyring entry")?;
    entry.set_password(token).context("store token in keychain")?;
    Ok(())
}

fn load_token() -> Result<String> {
    if let Ok(token) = std::env::var("ORBIT_TOKEN") {
        return Ok(token);
    }
    let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER).context("create keyring entry")?;
    entry.get_password().context("no token found — run `orbit login`")
}

fn delete_token() -> Result<()> {
    let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER).context("create keyring entry")?;
    let _ = entry.delete_password();
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// HTTP client helper
// ─────────────────────────────────────────────────────────────────────────────

struct Client {
    inner: reqwest::Client,
    base_url: String,
    token: Option<String>,
}

impl Client {
    fn new(base_url: String, token: Option<String>) -> Self {
        let inner = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("reqwest client build failed");
        Self {
            inner,
            base_url,
            token,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url.trim_end_matches('/'), path)
    }

    fn auth_header(&self) -> Option<String> {
        self.token.as_ref().map(|t| format!("Bearer {t}"))
    }

    async fn get(&self, path: &str) -> Result<Value> {
        let mut req = self.inner.get(self.url(path));
        if let Some(auth) = self.auth_header() {
            req = req.header("Authorization", auth);
        }
        let resp = req.send().await.context("HTTP GET failed")?;
        self.parse_response(resp).await
    }

    async fn post<B: Serialize>(&self, path: &str, body: &B) -> Result<Value> {
        let mut req = self.inner.post(self.url(path)).json(body);
        if let Some(auth) = self.auth_header() {
            req = req.header("Authorization", auth);
        }
        let resp = req.send().await.context("HTTP POST failed")?;
        self.parse_response(resp).await
    }

    async fn delete(&self, path: &str) -> Result<Value> {
        let mut req = self.inner.delete(self.url(path));
        if let Some(auth) = self.auth_header() {
            req = req.header("Authorization", auth);
        }
        let resp = req.send().await.context("HTTP DELETE failed")?;
        self.parse_response(resp).await
    }

    async fn patch<B: Serialize>(&self, path: &str, body: &B) -> Result<Value> {
        let mut req = self.inner.patch(self.url(path)).json(body);
        if let Some(auth) = self.auth_header() {
            req = req.header("Authorization", auth);
        }
        let resp = req.send().await.context("HTTP PATCH failed")?;
        self.parse_response(resp).await
    }

    async fn parse_response(&self, resp: reqwest::Response) -> Result<Value> {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            let msg = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v["error"].as_str().map(|s| s.to_string()))
                .unwrap_or_else(|| text.clone());
            bail!("API error {}: {}", status, msg);
        }
        if text.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).with_context(|| format!("parse response JSON: {text}"))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// CLI structure
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "orbit",
    about = "JottiCP control panel CLI",
    version = env!("CARGO_PKG_VERSION")
)]
struct Cli {
    /// Output JSON instead of table (for list commands)
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Authenticate with the JottiCP panel
    Login {
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        email: Option<String>,
        #[arg(long)]
        password: Option<String>,
        #[arg(long)]
        totp: Option<String>,
    },
    /// Clear stored authentication token
    Logout,
    /// Site management
    Sites {
        #[command(subcommand)]
        action: SitesCommands,
    },
    /// SSL certificate management
    Ssl {
        #[command(subcommand)]
        action: SslCommands,
    },
    /// Email account management
    Email {
        #[command(subcommand)]
        action: EmailCommands,
    },
    /// DNS record management
    Dns {
        #[command(subcommand)]
        action: DnsCommands,
    },
    /// Backup management
    Backup {
        #[command(subcommand)]
        action: BackupCommands,
    },
    /// Server management (admin only)
    Servers {
        #[command(subcommand)]
        action: ServersCommands,
    },
    /// CLI configuration
    Config {
        #[command(subcommand)]
        action: ConfigCommands,
    },
    /// Generate shell completion script
    Completion {
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[derive(Subcommand)]
enum SitesCommands {
    /// List sites
    List {
        #[arg(long)]
        server: Option<String>,
        #[arg(long, value_enum)]
        status: Option<SiteStatus>,
    },
    /// Create a new site
    Create {
        #[arg(long)]
        domain: String,
        #[arg(long, default_value = "8.3")]
        php: String,
        #[arg(long, default_value = "ols")]
        web_server: String,
        #[arg(long)]
        server: Option<String>,
    },
    /// Delete a site
    Delete {
        #[arg(long)]
        domain: String,
        #[arg(long)]
        confirm: bool,
    },
    /// Suspend a site
    Suspend {
        #[arg(long)]
        domain: String,
    },
    /// Unsuspend a site
    Unsuspend {
        #[arg(long)]
        domain: String,
    },
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum SiteStatus {
    Active,
    Suspended,
}

#[derive(Subcommand)]
enum SslCommands {
    /// Issue a Let's Encrypt certificate
    Issue {
        #[arg(long)]
        domain: String,
        /// Request wildcard certificate (Pro)
        #[arg(long)]
        wildcard: bool,
    },
    /// Upload a custom certificate (Pro)
    Upload {
        #[arg(long)]
        domain: String,
        #[arg(long)]
        cert: PathBuf,
        #[arg(long)]
        key: PathBuf,
    },
    /// Trigger immediate renewal of an existing certificate
    Renew {
        #[arg(long)]
        domain: String,
    },
}

#[derive(Subcommand)]
enum EmailCommands {
    /// List email accounts for a domain
    List {
        #[arg(long)]
        domain: String,
    },
    /// Create an email account
    Create {
        #[arg(long)]
        address: String,
        #[arg(long)]
        password: String,
        #[arg(long)]
        quota: Option<u64>,
    },
    /// Delete an email account
    Delete {
        #[arg(long)]
        address: String,
    },
}

#[derive(Subcommand)]
enum DnsCommands {
    /// List DNS records for a domain
    List {
        #[arg(long)]
        domain: String,
    },
    /// Add a DNS record
    Add {
        #[arg(long)]
        domain: String,
        #[arg(long, name = "type")]
        record_type: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        value: String,
        #[arg(long, default_value_t = 300)]
        ttl: u32,
    },
    /// Delete a DNS record
    Delete {
        #[arg(long)]
        domain: String,
        #[arg(long, name = "type")]
        record_type: String,
        #[arg(long)]
        name: String,
    },
}

#[derive(Subcommand)]
enum BackupCommands {
    /// Create a backup
    Create {
        #[arg(long)]
        domain: String,
        #[arg(long, default_value = "full")]
        backup_type: String,
    },
    /// List backups
    List {
        #[arg(long)]
        domain: String,
    },
    /// Restore a backup
    Restore {
        #[arg(long)]
        backup_id: String,
    },
}

#[derive(Subcommand)]
enum ServersCommands {
    /// List servers (admin only)
    List,
    /// Add a managed server (admin only)
    Add {
        #[arg(long)]
        label: String,
        #[arg(long)]
        ip: String,
    },
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Set the panel base URL
    SetUrl { url: String },
    /// Set an arbitrary config key (key=value syntax)
    Set {
        /// Config key to set (currently only "url" is supported)
        key: String,
        /// Value to assign
        value: String,
    },
    /// Show current configuration
    Show,
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum Shell {
    Bash,
    Zsh,
    Fish,
}

// ─────────────────────────────────────────────────────────────────────────────
// Table row types for display
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Tabled)]
struct SiteRow {
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "Domain")]
    domain: String,
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "PHP")]
    php: String,
    #[tabled(rename = "Web Server")]
    web_server: String,
    #[tabled(rename = "SSL")]
    ssl: String,
    #[tabled(rename = "Disk MB")]
    disk_mb: String,
    #[tabled(rename = "Created")]
    created: String,
}

#[derive(Tabled)]
struct EmailRow {
    #[tabled(rename = "Address")]
    address: String,
    #[tabled(rename = "Quota MB")]
    quota_mb: String,
    #[tabled(rename = "Used MB")]
    used_mb: String,
    #[tabled(rename = "Status")]
    status: String,
}

#[derive(Tabled)]
struct DnsRow {
    #[tabled(rename = "Name")]
    name: String,
    #[tabled(rename = "Type")]
    record_type: String,
    #[tabled(rename = "TTL")]
    ttl: String,
    #[tabled(rename = "Content")]
    content: String,
}

#[derive(Tabled)]
struct BackupRow {
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "Domain")]
    domain: String,
    #[tabled(rename = "Type")]
    backup_type: String,
    #[tabled(rename = "Size MB")]
    size_mb: String,
    #[tabled(rename = "Created")]
    created: String,
    #[tabled(rename = "Status")]
    status: String,
}

#[derive(Tabled)]
struct ServerRow {
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "Label")]
    label: String,
    #[tabled(rename = "IP")]
    ip: String,
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "Sites")]
    sites: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Output helpers
// ─────────────────────────────────────────────────────────────────────────────

fn print_success(msg: &str) {
    println!("{} {}", "✓".green().bold(), msg.green());
}

fn print_error(msg: &str) {
    eprintln!("{} {}", "✗".red().bold(), msg.red());
}

fn print_json(v: &Value) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

fn str_val(v: &Value, key: &str) -> String {
    v[key]
        .as_str()
        .map(|s| s.to_string())
        .or_else(|| v[key].as_i64().map(|n| n.to_string()))
        .or_else(|| v[key].as_f64().map(|n| n.to_string()))
        .or_else(|| v[key].as_bool().map(|b| b.to_string()))
        .unwrap_or_else(|| "-".to_string())
}

// ─────────────────────────────────────────────────────────────────────────────
// Prompt helpers
// ─────────────────────────────────────────────────────────────────────────────

fn prompt(label: &str, secret: bool) -> Result<String> {
    if secret {
        rpassword_prompt(label)
    } else {
        use std::io::{self, Write};
        print!("{label}: ");
        io::stdout().flush()?;
        let mut buf = String::new();
        io::stdin().read_line(&mut buf)?;
        Ok(buf.trim().to_string())
    }
}

fn rpassword_prompt(label: &str) -> Result<String> {
    rpassword::prompt_password(format!("{label}: "))
        .with_context(|| format!("failed to read secret input for '{label}'"))
}

fn confirm(msg: &str) -> Result<bool> {
    use std::io::{self, Write};
    print!("{} [y/N]: ", msg.yellow());
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf.trim().eq_ignore_ascii_case("y"))
}

// ─────────────────────────────────────────────────────────────────────────────
// Command handlers
// ─────────────────────────────────────────────────────────────────────────────

async fn cmd_login(
    url: Option<String>,
    email: Option<String>,
    password: Option<String>,
    totp_code: Option<String>,
) -> Result<()> {
    let url = match url {
        Some(u) => u,
        None => {
            let existing = get_base_url().unwrap_or_default();
            if !existing.is_empty() {
                existing
            } else {
                prompt("Panel URL (e.g. https://panel.example.com)", false)?
            }
        }
    };

    let email = match email {
        Some(e) => e,
        None => prompt("Email", false)?,
    };

    let password = match password {
        Some(p) => p,
        None => prompt("Password", true)?,
    };

    // Save URL first
    let mut cfg = load_config();
    cfg.url = Some(url.clone());
    save_config(&cfg)?;

    let client = Client::new(url, None);

    // Step 1: login
    let login_body = serde_json::json!({
        "email": email,
        "password": password,
    });

    let login_resp = client.post("/api/v1/auth/login", &login_body).await?;

    // If TOTP required
    let token = if login_resp["totp_required"].as_bool().unwrap_or(false) {
        let totp = match totp_code {
            Some(t) => t,
            None => prompt("TOTP code", false)?,
        };
        let session_token = login_resp["session_token"]
            .as_str()
            .unwrap_or_default()
            .to_string();

        let totp_body = serde_json::json!({
            "session_token": session_token,
            "code": totp,
        });

        let totp_resp = client.post("/api/v1/auth/totp", &totp_body).await?;
        totp_resp["token"]
            .as_str()
            .context("no token in TOTP response")?
            .to_string()
    } else {
        login_resp["token"]
            .as_str()
            .context("no token in login response")?
            .to_string()
    };

    store_token(&token)?;
    print_success(&format!("Logged in as {email}"));
    Ok(())
}

async fn cmd_logout() -> Result<()> {
    delete_token()?;
    print_success("Logged out");
    Ok(())
}

async fn cmd_sites_list(
    json: bool,
    server: Option<String>,
    status_filter: Option<SiteStatus>,
) -> Result<()> {
    let base_url = get_base_url()?;
    let token = load_token()?;
    let client = Client::new(base_url, Some(token));

    let mut path = "/api/v1/sites".to_string();
    let mut params: Vec<String> = vec![];
    if let Some(s) = &server {
        params.push(format!("server_id={s}"));
    }
    if let Some(s) = &status_filter {
        params.push(format!("status={}", format!("{s:?}").to_lowercase()));
    }
    if !params.is_empty() {
        path = format!("{path}?{}", params.join("&"));
    }

    let resp = client.get(&path).await?;

    let sites = resp["sites"]
        .as_array()
        .cloned()
        .unwrap_or_else(|| resp.as_array().cloned().unwrap_or_default());

    if json {
        print_json(&serde_json::json!(sites));
        return Ok(());
    }

    let rows: Vec<SiteRow> = sites
        .iter()
        .map(|s| SiteRow {
            id: str_val(s, "id").chars().take(8).collect(),
            domain: str_val(s, "domain"),
            status: str_val(s, "status"),
            php: str_val(s, "php_version"),
            web_server: str_val(s, "web_server"),
            ssl: str_val(s, "ssl_status"),
            disk_mb: str_val(s, "disk_mb"),
            created: str_val(s, "created_at").chars().take(10).collect(),
        })
        .collect();

    if rows.is_empty() {
        println!("No sites found.");
    } else {
        println!("{}", Table::new(rows));
    }
    Ok(())
}

async fn cmd_sites_create(
    domain: String,
    php: String,
    web_server: String,
    server: Option<String>,
) -> Result<()> {
    let base_url = get_base_url()?;
    let token = load_token()?;
    let client = Client::new(base_url, Some(token));

    let mut body = serde_json::json!({
        "domain": domain,
        "php_version": php,
        "web_server": web_server,
    });
    if let Some(s) = server {
        body["server_id"] = s.into();
    }

    let resp = client.post("/api/v1/sites", &body).await?;
    print_success(&format!(
        "Site '{}' created (ID: {})",
        domain,
        resp["id"].as_str().unwrap_or("-")
    ));
    Ok(())
}

async fn cmd_sites_delete(domain: String, confirm_flag: bool) -> Result<()> {
    if !confirm_flag {
        let ok = confirm(&format!("Delete site '{domain}'? This action is irreversible."))?;
        if !ok {
            println!("Aborted.");
            return Ok(());
        }
    }

    let base_url = get_base_url()?;
    let token = load_token()?;
    let client = Client::new(base_url, Some(token));

    // Resolve domain to ID
    let resp = client.get(&format!("/api/v1/sites?domain={domain}")).await?;
    let site_id = resp["sites"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|s| s["id"].as_str())
        .context("site not found")?
        .to_string();

    client.delete(&format!("/api/v1/sites/{site_id}")).await?;
    print_success(&format!("Site '{domain}' deleted"));
    Ok(())
}

async fn cmd_sites_suspend(domain: String) -> Result<()> {
    let base_url = get_base_url()?;
    let token = load_token()?;
    let client = Client::new(base_url, Some(token));

    let resp = client.get(&format!("/api/v1/sites?domain={domain}")).await?;
    let site_id = resp["sites"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|s| s["id"].as_str())
        .context("site not found")?
        .to_string();

    client
        .post(
            &format!("/api/v1/sites/{site_id}/suspend"),
            &serde_json::json!({}),
        )
        .await?;
    print_success(&format!("Site '{domain}' suspended"));
    Ok(())
}

async fn cmd_sites_unsuspend(domain: String) -> Result<()> {
    let base_url = get_base_url()?;
    let token = load_token()?;
    let client = Client::new(base_url, Some(token));

    let resp = client.get(&format!("/api/v1/sites?domain={domain}")).await?;
    let site_id = resp["sites"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|s| s["id"].as_str())
        .context("site not found")?
        .to_string();

    client
        .post(
            &format!("/api/v1/sites/{site_id}/unsuspend"),
            &serde_json::json!({}),
        )
        .await?;
    print_success(&format!("Site '{domain}' unsuspended"));
    Ok(())
}

async fn cmd_ssl_issue(domain: String, wildcard: bool) -> Result<()> {
    let base_url = get_base_url()?;
    let token = load_token()?;
    let client = Client::new(base_url, Some(token));

    let body = serde_json::json!({ "domain": domain, "wildcard": wildcard });
    let resp = client.post("/api/v1/ssl/issue", &body).await?;
    print_success(&format!(
        "SSL issuance started for '{}' (job_id: {})",
        domain,
        resp["job_id"].as_str().unwrap_or("-")
    ));
    Ok(())
}

async fn cmd_ssl_renew(domain: String) -> Result<()> {
    let base_url = get_base_url()?;
    let token = load_token()?;
    let client = Client::new(base_url, Some(token));

    // Resolve domain to cert UUID
    let certs_resp = client
        .get(&format!("/api/v1/ssl?domain={domain}"))
        .await
        .with_context(|| "panel unreachable — check your configured URL".to_string())?;

    let cert_id = certs_resp["certs"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|c| c["id"].as_str())
        .with_context(|| format!("no SSL certificate found for '{domain}'"))?
        .to_string();

    let resp = client
        .post(&format!("/api/v1/ssl/{cert_id}/renew"), &serde_json::json!({}))
        .await?;
    print_success(&format!(
        "SSL renewal triggered for '{}' (job_id: {})",
        domain,
        resp["job_id"].as_str().unwrap_or("-")
    ));
    Ok(())
}

async fn cmd_ssl_upload(domain: String, cert: PathBuf, key: PathBuf) -> Result<()> {
    let base_url = get_base_url()?;
    let token = load_token()?;
    let client = Client::new(base_url, Some(token));

    let cert_pem = fs::read_to_string(&cert)
        .with_context(|| format!("read cert file {}", cert.display()))?;
    let key_pem = fs::read_to_string(&key)
        .with_context(|| format!("read key file {}", key.display()))?;

    let body = serde_json::json!({
        "domain": domain,
        "cert_pem": cert_pem,
        "key_pem": key_pem,
    });
    client.post("/api/v1/ssl/upload", &body).await?;
    print_success(&format!("SSL certificate uploaded for '{domain}'"));
    Ok(())
}

async fn cmd_email_list(json: bool, domain: String) -> Result<()> {
    let base_url = get_base_url()?;
    let token = load_token()?;
    let client = Client::new(base_url, Some(token));

    let resp = client
        .get(&format!("/api/v1/email/accounts?domain={domain}"))
        .await?;

    let accounts = resp["accounts"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    if json {
        print_json(&serde_json::json!(accounts));
        return Ok(());
    }

    let rows: Vec<EmailRow> = accounts
        .iter()
        .map(|a| EmailRow {
            address: str_val(a, "email"),
            quota_mb: str_val(a, "quota_mb"),
            used_mb: str_val(a, "used_mb"),
            status: str_val(a, "status"),
        })
        .collect();

    if rows.is_empty() {
        println!("No email accounts for '{domain}'.");
    } else {
        println!("{}", Table::new(rows));
    }
    Ok(())
}

async fn cmd_email_create(
    address: String,
    password: String,
    quota: Option<u64>,
) -> Result<()> {
    let base_url = get_base_url()?;
    let token = load_token()?;
    let client = Client::new(base_url, Some(token));

    let parts: Vec<&str> = address.splitn(2, '@').collect();
    if parts.len() != 2 {
        bail!("address must be user@domain");
    }
    let (username, domain) = (parts[0], parts[1]);

    let body = serde_json::json!({
        "username": username,
        "domain": domain,
        "password": password,
        "quota_mb": quota.unwrap_or(1024),
    });
    client.post("/api/v1/email/accounts", &body).await?;
    print_success(&format!("Email account '{address}' created"));
    Ok(())
}

async fn cmd_email_delete(address: String) -> Result<()> {
    let base_url = get_base_url()?;
    let token = load_token()?;
    let client = Client::new(base_url, Some(token));

    let ok = confirm(&format!("Delete email account '{address}'?"))?;
    if !ok {
        println!("Aborted.");
        return Ok(());
    }

    client
        .delete(&format!("/api/v1/email/accounts/{address}"))
        .await?;
    print_success(&format!("Email account '{address}' deleted"));
    Ok(())
}

async fn cmd_dns_list(json: bool, domain: String) -> Result<()> {
    let base_url = get_base_url()?;
    let token = load_token()?;
    let client = Client::new(base_url, Some(token));

    let resp = client
        .get(&format!("/api/v1/dns/zones/{domain}/records"))
        .await?;

    let rrsets = resp["rrsets"].as_array().cloned().unwrap_or_default();

    if json {
        print_json(&serde_json::json!(rrsets));
        return Ok(());
    }

    let mut rows: Vec<DnsRow> = vec![];
    for rr in &rrsets {
        let name = str_val(rr, "name");
        let rtype = str_val(rr, "type");
        let ttl = str_val(rr, "ttl");
        if let Some(records) = rr["records"].as_array() {
            for r in records {
                rows.push(DnsRow {
                    name: name.clone(),
                    record_type: rtype.clone(),
                    ttl: ttl.clone(),
                    content: r["content"].as_str().unwrap_or("-").to_string(),
                });
            }
        }
    }

    if rows.is_empty() {
        println!("No DNS records for '{domain}'.");
    } else {
        println!("{}", Table::new(rows));
    }
    Ok(())
}

async fn cmd_dns_add(
    domain: String,
    record_type: String,
    name: String,
    value: String,
    ttl: u32,
) -> Result<()> {
    let base_url = get_base_url()?;
    let token = load_token()?;
    let client = Client::new(base_url, Some(token));

    let body = serde_json::json!({
        "name": name,
        "type": record_type,
        "ttl": ttl,
        "records": [{ "content": value, "disabled": false }],
    });

    client
        .post(&format!("/api/v1/dns/zones/{domain}/records"), &body)
        .await?;
    print_success(&format!("DNS record added: {name} {record_type} → {value}"));
    Ok(())
}

async fn cmd_dns_delete(domain: String, record_type: String, name: String) -> Result<()> {
    let base_url = get_base_url()?;
    let token = load_token()?;
    let client = Client::new(base_url, Some(token));

    client
        .delete(&format!(
            "/api/v1/dns/zones/{domain}/records/{name}/{record_type}"
        ))
        .await?;
    print_success(&format!("DNS record deleted: {name} {record_type}"));
    Ok(())
}

async fn cmd_backup_create(domain: String, backup_type: String) -> Result<()> {
    let base_url = get_base_url()?;
    let token = load_token()?;
    let client = Client::new(base_url, Some(token));

    let body = serde_json::json!({ "domain": domain, "type": backup_type });
    let resp = client.post("/api/v1/backups", &body).await?;
    print_success(&format!(
        "Backup started for '{}' (job_id: {})",
        domain,
        resp["job_id"].as_str().unwrap_or("-")
    ));
    Ok(())
}

async fn cmd_backup_list(json: bool, domain: String) -> Result<()> {
    let base_url = get_base_url()?;
    let token = load_token()?;
    let client = Client::new(base_url, Some(token));

    let resp = client
        .get(&format!("/api/v1/backups?domain={domain}"))
        .await?;

    let backups = resp["backups"].as_array().cloned().unwrap_or_default();

    if json {
        print_json(&serde_json::json!(backups));
        return Ok(());
    }

    let rows: Vec<BackupRow> = backups
        .iter()
        .map(|b| BackupRow {
            id: str_val(b, "id").chars().take(8).collect(),
            domain: str_val(b, "domain"),
            backup_type: str_val(b, "type"),
            size_mb: str_val(b, "size_mb"),
            created: str_val(b, "created_at").chars().take(10).collect(),
            status: str_val(b, "status"),
        })
        .collect();

    if rows.is_empty() {
        println!("No backups found for '{domain}'.");
    } else {
        println!("{}", Table::new(rows));
    }
    Ok(())
}

async fn cmd_backup_restore(backup_id: String) -> Result<()> {
    let base_url = get_base_url()?;
    let token = load_token()?;
    let client = Client::new(base_url, Some(token));

    let ok = confirm(&format!("Restore backup '{backup_id}'? This will overwrite existing data."))?;
    if !ok {
        println!("Aborted.");
        return Ok(());
    }

    let resp = client
        .post(
            &format!("/api/v1/backups/{backup_id}/restore"),
            &serde_json::json!({}),
        )
        .await?;
    print_success(&format!(
        "Restore started (job_id: {})",
        resp["job_id"].as_str().unwrap_or("-")
    ));
    Ok(())
}

async fn cmd_servers_list(json: bool) -> Result<()> {
    let base_url = get_base_url()?;
    let token = load_token()?;
    let client = Client::new(base_url, Some(token));

    let resp = client.get("/api/v1/servers").await?;
    let servers = resp["servers"].as_array().cloned().unwrap_or_default();

    if json {
        print_json(&serde_json::json!(servers));
        return Ok(());
    }

    let rows: Vec<ServerRow> = servers
        .iter()
        .map(|s| ServerRow {
            id: str_val(s, "id").chars().take(8).collect(),
            label: str_val(s, "label"),
            ip: str_val(s, "ip_address"),
            status: str_val(s, "status"),
            sites: str_val(s, "site_count"),
        })
        .collect();

    if rows.is_empty() {
        println!("No servers found.");
    } else {
        println!("{}", Table::new(rows));
    }
    Ok(())
}

async fn cmd_servers_add(label: String, ip: String) -> Result<()> {
    let base_url = get_base_url()?;
    let token = load_token()?;
    let client = Client::new(base_url, Some(token));

    let body = serde_json::json!({ "label": label, "ip_address": ip });
    let resp = client.post("/api/v1/servers", &body).await?;
    print_success(&format!(
        "Server '{}' added (ID: {})",
        label,
        resp["id"].as_str().unwrap_or("-")
    ));
    Ok(())
}

fn cmd_config_set_url(url: String) -> Result<()> {
    let mut cfg = load_config();
    cfg.url = Some(url.clone());
    save_config(&cfg)?;
    print_success(&format!("Panel URL set to '{url}'"));
    Ok(())
}

/// Generic `orbit config set KEY VALUE` handler.
/// Currently the only supported key is "url".
fn cmd_config_set(key: String, value: String) -> Result<()> {
    match key.as_str() {
        "url" => cmd_config_set_url(value),
        other => bail!("unknown config key '{}' — supported keys: url", other),
    }
}

fn cmd_config_show() -> Result<()> {
    let cfg = load_config();
    println!(
        "url = {}",
        cfg.url.as_deref().unwrap_or("(not set)")
    );
    println!(
        "token = {}",
        load_token()
            .map(|_| "(stored in keychain)".to_string())
            .unwrap_or_else(|_| "(not set)".to_string())
    );
    let path = config_path();
    println!("config_file = {}", path.display());
    Ok(())
}

fn cmd_completion(shell: Shell) {
    let mut cmd = Cli::command();
    match shell {
        Shell::Bash => {
            clap_complete::generate(
                clap_complete::shells::Bash,
                &mut cmd,
                "orbit",
                &mut std::io::stdout(),
            );
        }
        Shell::Zsh => {
            clap_complete::generate(
                clap_complete::shells::Zsh,
                &mut cmd,
                "orbit",
                &mut std::io::stdout(),
            );
        }
        Shell::Fish => {
            clap_complete::generate(
                clap_complete::shells::Fish,
                &mut cmd,
                "orbit",
                &mut std::io::stdout(),
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Main
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let cli = Cli::parse();
    let json = cli.json;

    let result: Result<()> = match cli.command {
        Commands::Login {
            url,
            email,
            password,
            totp,
        } => cmd_login(url, email, password, totp).await,

        Commands::Logout => cmd_logout().await,

        Commands::Sites { action } => match action {
            SitesCommands::List { server, status } => {
                cmd_sites_list(json, server, status).await
            }
            SitesCommands::Create {
                domain,
                php,
                web_server,
                server,
            } => cmd_sites_create(domain, php, web_server, server).await,
            SitesCommands::Delete { domain, confirm } => {
                cmd_sites_delete(domain, confirm).await
            }
            SitesCommands::Suspend { domain } => cmd_sites_suspend(domain).await,
            SitesCommands::Unsuspend { domain } => cmd_sites_unsuspend(domain).await,
        },

        Commands::Ssl { action } => match action {
            SslCommands::Issue { domain, wildcard } => cmd_ssl_issue(domain, wildcard).await,
            SslCommands::Upload { domain, cert, key } => {
                cmd_ssl_upload(domain, cert, key).await
            }
            SslCommands::Renew { domain } => cmd_ssl_renew(domain).await,
        },

        Commands::Email { action } => match action {
            EmailCommands::List { domain } => cmd_email_list(json, domain).await,
            EmailCommands::Create {
                address,
                password,
                quota,
            } => cmd_email_create(address, password, quota).await,
            EmailCommands::Delete { address } => cmd_email_delete(address).await,
        },

        Commands::Dns { action } => match action {
            DnsCommands::List { domain } => cmd_dns_list(json, domain).await,
            DnsCommands::Add {
                domain,
                record_type,
                name,
                value,
                ttl,
            } => cmd_dns_add(domain, record_type, name, value, ttl).await,
            DnsCommands::Delete {
                domain,
                record_type,
                name,
            } => cmd_dns_delete(domain, record_type, name).await,
        },

        Commands::Backup { action } => match action {
            BackupCommands::Create {
                domain,
                backup_type,
            } => cmd_backup_create(domain, backup_type).await,
            BackupCommands::List { domain } => cmd_backup_list(json, domain).await,
            BackupCommands::Restore { backup_id } => cmd_backup_restore(backup_id).await,
        },

        Commands::Servers { action } => match action {
            ServersCommands::List => cmd_servers_list(json).await,
            ServersCommands::Add { label, ip } => cmd_servers_add(label, ip).await,
        },

        Commands::Config { action } => match action {
            ConfigCommands::SetUrl { url } => cmd_config_set_url(url),
            ConfigCommands::Set { key, value } => cmd_config_set(key, value),
            ConfigCommands::Show => cmd_config_show(),
        },

        Commands::Completion { shell } => {
            cmd_completion(shell);
            Ok(())
        }
    };

    if let Err(e) = result {
        print_error(&format!("{e:#}"));
        std::process::exit(1);
    }
}
