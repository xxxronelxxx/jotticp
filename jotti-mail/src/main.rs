/// jotti-mail — Postfix + Dovecot + rspamd management daemon
///
/// Listens on 127.0.0.1:5354 (internal only).
/// All filesystem writes are atomic where possible (write to tmp, rename).
/// postmap and postfix reload are executed synchronously via tokio::process.

use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use regex::Regex;

use anyhow::{Context, Result};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::Mutex,
};
use tracing::{error, info, instrument, warn};

// ─────────────────────────────────────────────────────────────────────────────
// Configuration
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Config {
    listen_addr: SocketAddr,
    virtual_domains_file: PathBuf,
    virtual_mailbox_maps_file: PathBuf,
    virtual_alias_maps_file: PathBuf,
    dovecot_users_file: PathBuf,
    vmail_base: PathBuf,
    dkim_key_dir: PathBuf,
    sieve_base: PathBuf,
}

impl Config {
    fn from_env() -> Result<Self> {
        let listen_addr = std::env::var("ORBIT_MAIL_LISTEN")
            .unwrap_or_else(|_| "127.0.0.1:5354".to_string())
            .parse()
            .context("invalid ORBIT_MAIL_LISTEN")?;
        Ok(Self {
            listen_addr,
            virtual_domains_file: PathBuf::from(
                std::env::var("POSTFIX_VIRTUAL_DOMAINS")
                    .unwrap_or_else(|_| "/etc/postfix/virtual_domains".into()),
            ),
            virtual_mailbox_maps_file: PathBuf::from(
                std::env::var("POSTFIX_VIRTUAL_MAILBOX_MAPS")
                    .unwrap_or_else(|_| "/etc/postfix/virtual_mailbox_maps".into()),
            ),
            virtual_alias_maps_file: PathBuf::from(
                std::env::var("POSTFIX_VIRTUAL_ALIAS_MAPS")
                    .unwrap_or_else(|_| "/etc/postfix/virtual_alias_maps".into()),
            ),
            dovecot_users_file: PathBuf::from(
                std::env::var("DOVECOT_USERS_FILE")
                    .unwrap_or_else(|_| "/etc/dovecot/users".into()),
            ),
            vmail_base: PathBuf::from(
                std::env::var("VMAIL_BASE").unwrap_or_else(|_| "/var/mail/vhosts".into()),
            ),
            dkim_key_dir: PathBuf::from(
                std::env::var("DKIM_KEY_DIR")
                    .unwrap_or_else(|_| "/etc/rspamd/dkim".into()),
            ),
            sieve_base: PathBuf::from(
                std::env::var("SIEVE_BASE").unwrap_or_else(|_| "/var/mail/sieve".into()),
            ),
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared state — mutex protects concurrent file writes
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct AppState {
    config: Arc<Config>,
    /// Serialize all writes to postfix/dovecot config files
    write_lock: Arc<Mutex<()>>,
}

impl AppState {
    fn new(config: Config) -> Self {
        Self {
            config: Arc::new(config),
            write_lock: Arc::new(Mutex::new(())),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Error type
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn internal(msg: impl Into<String>) -> Self {
        let m = msg.into();
        error!(error = %m, "internal error");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, m)
    }

    fn not_found(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, msg)
    }

    fn bad_request(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, msg)
    }

    fn conflict(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, msg)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = json!({ "error": self.message });
        (self.status, Json(body)).into_response()
    }
}

type ApiResult<T> = std::result::Result<T, ApiError>;

// ─────────────────────────────────────────────────────────────────────────────
// Process helpers
// ─────────────────────────────────────────────────────────────────────────────

async fn run_cmd(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .await
        .with_context(|| format!("failed to spawn {program}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("{program} exited with status {:?}: {stderr}", output.status.code());
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

async fn postmap(file: &PathBuf) -> Result<()> {
    run_cmd("postmap", &[file.to_str().unwrap()])
        .await
        .with_context(|| format!("postmap {}", file.display()))?;
    Ok(())
}

async fn postfix_reload() -> Result<()> {
    run_cmd("postfix", &["reload"])
        .await
        .context("postfix reload")?;
    Ok(())
}

/// Hash password using doveadm pw -s SHA512-CRYPT
async fn hash_password_dovecot(password: &str) -> Result<String> {
    let output = Command::new("doveadm")
        .args(["pw", "-s", "SHA512-CRYPT", "-p", password])
        .output()
        .await
        .context("failed to spawn doveadm pw")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("doveadm pw failed: {stderr}");
    }

    let hashed = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_string();
    Ok(hashed)
}

// ─────────────────────────────────────────────────────────────────────────────
// File helpers — read, modify, write with line-by-line manipulation
// ─────────────────────────────────────────────────────────────────────────────

async fn read_lines(path: &PathBuf) -> Result<Vec<String>> {
    match fs::read_to_string(path).await {
        Ok(content) => Ok(content
            .lines()
            .map(|l| l.to_string())
            .collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

async fn write_lines(path: &PathBuf, lines: &[String]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    let mut file = fs::File::create(&tmp)
        .await
        .with_context(|| format!("create tmp file {}", tmp.display()))?;
    for line in lines {
        file.write_all(line.as_bytes()).await?;
        file.write_all(b"\n").await?;
    }
    // flush() only drains the tokio write buffer into the kernel page cache.
    // sync_all() also calls fsync(2) so the data reaches stable storage before
    // we rename — this prevents a corrupt-but-zero-byte file if the host
    // crashes between the write and the rename.
    file.flush().await?;
    file.sync_all().await?;
    drop(file);
    fs::rename(&tmp, path)
        .await
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

async fn append_line(path: &PathBuf, line: &str) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .with_context(|| format!("open {} for append", path.display()))?;
    file.write_all(line.as_bytes()).await?;
    file.write_all(b"\n").await?;
    file.flush().await?;
    Ok(())
}

async fn remove_lines_matching(path: &PathBuf, prefix: &str) -> Result<usize> {
    let lines = read_lines(path).await?;
    let before = lines.len();
    let filtered: Vec<String> = lines
        .into_iter()
        .filter(|l| !l.starts_with(prefix) && !l.is_empty() || l.is_empty())
        .collect();
    let removed = before - filtered.len();
    write_lines(path, &filtered).await?;
    Ok(removed)
}

// ─────────────────────────────────────────────────────────────────────────────
// DTOs
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct AddDomainRequest {
    domain: String,
}

#[derive(Debug, Deserialize)]
struct CreateAccountRequest {
    domain: String,
    username: String,
    password: String,
    quota_mb: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ChangePasswordRequest {
    password: String,
}

#[derive(Debug, Deserialize)]
struct CreateForwarderRequest {
    from_address: String,
    to_address: String,
}

#[derive(Debug, Deserialize)]
struct CreateAutoresponderRequest {
    email: String,
    subject: String,
    body: String,
    start_date: Option<String>,
    end_date: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Handlers — Domains
// ─────────────────────────────────────────────────────────────────────────────

#[instrument(skip(state, body))]
async fn add_domain(
    State(state): State<AppState>,
    Json(body): Json<AddDomainRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let domain = body.domain.to_lowercase();
    if domain.is_empty() {
        return Err(ApiError::bad_request("domain is required"));
    }

    let _lock = state.write_lock.lock().await;

    // Check if already present
    let lines = read_lines(&state.config.virtual_domains_file)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    if lines.iter().any(|l| l.trim() == domain) {
        return Err(ApiError::conflict(format!("domain '{domain}' already exists")));
    }

    append_line(&state.config.virtual_domains_file, &domain)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    // Create vmail directory for domain
    let domain_dir = state.config.vmail_base.join(&domain);
    fs::create_dir_all(&domain_dir)
        .await
        .map_err(|e| ApiError::internal(format!("failed to create vmail dir: {e}")))?;

    postfix_reload()
        .await
        .map_err(|e| ApiError::internal(format!("postfix reload failed: {e}")))?;

    info!(domain = %domain, "virtual domain added");
    Ok((StatusCode::CREATED, Json(json!({ "domain": domain, "status": "created" }))))
}

#[instrument(skip(state))]
async fn delete_domain(
    State(state): State<AppState>,
    Path(domain): Path<String>,
) -> ApiResult<StatusCode> {
    let domain = domain.to_lowercase();
    let _lock = state.write_lock.lock().await;

    let removed = remove_lines_matching(&state.config.virtual_domains_file, &domain)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    if removed == 0 {
        return Err(ApiError::not_found(format!("domain '{domain}' not found")));
    }

    postfix_reload()
        .await
        .map_err(|e| ApiError::internal(format!("postfix reload failed: {e}")))?;

    info!(domain = %domain, "virtual domain deleted");
    Ok(StatusCode::NO_CONTENT)
}

// ─────────────────────────────────────────────────────────────────────────────
// Handlers — Email Accounts
// ─────────────────────────────────────────────────────────────────────────────

#[instrument(skip(state, body))]
async fn create_account(
    State(state): State<AppState>,
    Json(body): Json<CreateAccountRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let domain = body.domain.to_lowercase();
    let username = body.username.to_lowercase();
    let email = format!("{username}@{domain}");
    let quota_mb = body.quota_mb.unwrap_or(1024);

    if username.is_empty() || domain.is_empty() {
        return Err(ApiError::bad_request("username and domain are required"));
    }
    if body.password.len() < 8 {
        return Err(ApiError::bad_request("password must be at least 8 characters"));
    }

    let _lock = state.write_lock.lock().await;

    // Check if account already exists in dovecot users
    let users = read_lines(&state.config.dovecot_users_file)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if users.iter().any(|l| l.starts_with(&format!("{email}:"))) {
        return Err(ApiError::conflict(format!("account '{email}' already exists")));
    }

    // Hash password via doveadm
    let hashed = hash_password_dovecot(&body.password)
        .await
        .map_err(|e| ApiError::internal(format!("password hashing failed: {e}")))?;

    let home_dir = format!("/var/mail/vhosts/{domain}/{username}");

    // Append to dovecot users file
    // Format: username@domain:{hashed_pw}:::{home_dir}::userdb_quota_rule=*:storage={quota_mb}M
    let dovecot_line = format!(
        "{email}:{hashed}:::{home_dir}::userdb_quota_rule=*:storage={quota_mb}M"
    );
    append_line(&state.config.dovecot_users_file, &dovecot_line)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    // Append to postfix virtual_mailbox_maps
    let mailbox_line = format!("{email} {domain}/{username}/");
    append_line(&state.config.virtual_mailbox_maps_file, &mailbox_line)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    // postmap
    postmap(&state.config.virtual_mailbox_maps_file)
        .await
        .map_err(|e| ApiError::internal(format!("postmap failed: {e}")))?;

    // Create Maildir
    let maildir_path = state.config.vmail_base.join(&domain).join(&username);
    run_cmd(
        "maildirmake.dovecot",
        &[maildir_path.to_str().unwrap()],
    )
    .await
    .map_err(|e| ApiError::internal(format!("maildirmake.dovecot failed: {e}")))?;

    info!(email = %email, quota_mb, "email account created");
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "email": email,
            "home": home_dir,
            "quota_mb": quota_mb,
            "status": "created"
        })),
    ))
}

#[instrument(skip(state))]
async fn delete_account(
    State(state): State<AppState>,
    Path(email): Path<String>,
) -> ApiResult<StatusCode> {
    let email = email.to_lowercase();
    let _lock = state.write_lock.lock().await;

    let removed_users = remove_lines_matching(&state.config.dovecot_users_file, &format!("{email}:"))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if removed_users == 0 {
        return Err(ApiError::not_found(format!("account '{email}' not found")));
    }

    remove_lines_matching(&state.config.virtual_mailbox_maps_file, &email)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    postmap(&state.config.virtual_mailbox_maps_file)
        .await
        .map_err(|e| ApiError::internal(format!("postmap failed: {e}")))?;

    info!(email = %email, "email account deleted");
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip(state, body))]
async fn change_password(
    State(state): State<AppState>,
    Path(email): Path<String>,
    Json(body): Json<ChangePasswordRequest>,
) -> ApiResult<Json<Value>> {
    let email = email.to_lowercase();
    if body.password.len() < 8 {
        return Err(ApiError::bad_request("password must be at least 8 characters"));
    }

    let _lock = state.write_lock.lock().await;

    let hashed = hash_password_dovecot(&body.password)
        .await
        .map_err(|e| ApiError::internal(format!("password hashing failed: {e}")))?;

    let mut lines = read_lines(&state.config.dovecot_users_file)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let prefix = format!("{email}:");
    let mut found = false;
    for line in &mut lines {
        if line.starts_with(&prefix) {
            // Replace password field (second colon-delimited field)
            let parts: Vec<&str> = line.splitn(3, ':').collect();
            if parts.len() >= 3 {
                *line = format!("{email}:{hashed}:{}", parts[2]);
                found = true;
                break;
            }
        }
    }

    if !found {
        return Err(ApiError::not_found(format!("account '{email}' not found")));
    }

    write_lines(&state.config.dovecot_users_file, &lines)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    info!(email = %email, "password changed");
    Ok(Json(json!({ "email": email, "status": "password_changed" })))
}

// ─────────────────────────────────────────────────────────────────────────────
// Handlers — Forwarders
// ─────────────────────────────────────────────────────────────────────────────

#[instrument(skip(state, body))]
async fn create_forwarder(
    State(state): State<AppState>,
    Json(body): Json<CreateForwarderRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let from = body.from_address.to_lowercase();
    let to = body.to_address.to_lowercase();

    if from.is_empty() || to.is_empty() {
        return Err(ApiError::bad_request("from_address and to_address are required"));
    }
    if from == to {
        return Err(ApiError::bad_request("from_address and to_address must be different"));
    }

    let _lock = state.write_lock.lock().await;

    let lines = read_lines(&state.config.virtual_alias_maps_file)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if lines.iter().any(|l| l.starts_with(&format!("{from} "))) {
        return Err(ApiError::conflict(format!("forwarder for '{from}' already exists")));
    }

    let alias_line = format!("{from} {to}");
    append_line(&state.config.virtual_alias_maps_file, &alias_line)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    postmap(&state.config.virtual_alias_maps_file)
        .await
        .map_err(|e| ApiError::internal(format!("postmap failed: {e}")))?;

    postfix_reload()
        .await
        .map_err(|e| ApiError::internal(format!("postfix reload failed: {e}")))?;

    info!(from = %from, to = %to, "forwarder created");
    Ok((
        StatusCode::CREATED,
        Json(json!({ "from": from, "to": to, "status": "created" })),
    ))
}

#[instrument(skip(state))]
async fn delete_forwarder(
    State(state): State<AppState>,
    Path(from_address): Path<String>,
) -> ApiResult<StatusCode> {
    let from = from_address.to_lowercase();
    let _lock = state.write_lock.lock().await;

    let removed = remove_lines_matching(
        &state.config.virtual_alias_maps_file,
        &format!("{from} "),
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    if removed == 0 {
        return Err(ApiError::not_found(format!("forwarder for '{from}' not found")));
    }

    postmap(&state.config.virtual_alias_maps_file)
        .await
        .map_err(|e| ApiError::internal(format!("postmap failed: {e}")))?;

    postfix_reload()
        .await
        .map_err(|e| ApiError::internal(format!("postfix reload failed: {e}")))?;

    info!(from = %from, "forwarder deleted");
    Ok(StatusCode::NO_CONTENT)
}

// ─────────────────────────────────────────────────────────────────────────────
// Handlers — Autoresponders (Sieve vacation script)
// ─────────────────────────────────────────────────────────────────────────────

fn build_vacation_sieve(
    subject: &str,
    body: &str,
    start_date: Option<&str>,
    end_date: Option<&str>,
) -> String {
    let date_check = match (start_date, end_date) {
        (Some(start), Some(end)) => format!(
            r#"  if not (currentdate :is "date" "{start}" or currentdate :value "ge" "date" "{start}") {{
    stop;
  }}
  if currentdate :value "gt" "date" "{end}" {{
    stop;
  }}
"#
        ),
        (Some(start), None) => format!(
            r#"  if not currentdate :value "ge" "date" "{start}" {{
    stop;
  }}
"#
        ),
        (None, Some(end)) => format!(
            r#"  if currentdate :value "gt" "date" "{end}" {{
    stop;
  }}
"#
        ),
        (None, None) => String::new(),
    };

    format!(
        r#"require ["vacation", "date", "relational"];
if header :contains "X-Spam-Flag" "YES" {{
  stop;
}}
{date_check}vacation
  :days 1
  :subject "{subject}"
  "{body}";
"#
    )
}

#[instrument(skip(state, body))]
async fn create_autoresponder(
    State(state): State<AppState>,
    Json(body): Json<CreateAutoresponderRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let email = body.email.to_lowercase();

    // email must be user@domain
    let parts: Vec<&str> = email.splitn(2, '@').collect();
    if parts.len() != 2 {
        return Err(ApiError::bad_request("email must be user@domain"));
    }
    let (user, domain) = (parts[0], parts[1]);

    let sieve_dir = state.config.sieve_base.join(domain).join(user);
    fs::create_dir_all(&sieve_dir)
        .await
        .map_err(|e| ApiError::internal(format!("failed to create sieve dir: {e}")))?;

    let sieve_script = build_vacation_sieve(
        &body.subject,
        &body.body,
        body.start_date.as_deref(),
        body.end_date.as_deref(),
    );

    let sieve_file = sieve_dir.join("vacation.sieve");
    let mut file = fs::File::create(&sieve_file)
        .await
        .map_err(|e| ApiError::internal(format!("failed to write sieve script: {e}")))?;
    file.write_all(sieve_script.as_bytes())
        .await
        .map_err(|e| ApiError::internal(format!("failed to write sieve content: {e}")))?;
    file.flush().await.ok();
    drop(file);

    // Compile with sievec
    match run_cmd("sievec", &[sieve_file.to_str().unwrap()]).await {
        Ok(_) => {}
        Err(e) => {
            warn!(email = %email, error = %e, "sievec compilation failed (non-fatal, will try at runtime)");
        }
    }

    info!(email = %email, "autoresponder created");
    Ok((
        StatusCode::CREATED,
        Json(json!({ "email": email, "status": "created" })),
    ))
}

#[instrument(skip(state))]
async fn delete_autoresponder(
    State(state): State<AppState>,
    Path(email): Path<String>,
) -> ApiResult<StatusCode> {
    let email = email.to_lowercase();
    let parts: Vec<&str> = email.splitn(2, '@').collect();
    if parts.len() != 2 {
        return Err(ApiError::bad_request("email must be user@domain"));
    }
    let (user, domain) = (parts[0], parts[1]);

    let sieve_dir = state.config.sieve_base.join(domain).join(user);
    let sieve_file = sieve_dir.join("vacation.sieve");
    let svbin_file = sieve_dir.join("vacation.svbin");

    for f in [&sieve_file, &svbin_file] {
        match fs::remove_file(f).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(ApiError::internal(format!(
                    "failed to remove {}: {e}",
                    f.display()
                )));
            }
        }
    }

    info!(email = %email, "autoresponder deleted");
    Ok(StatusCode::NO_CONTENT)
}

// ─────────────────────────────────────────────────────────────────────────────
// Step 3.9 — Bulk email / rate limiting
// ─────────────────────────────────────────────────────────────────────────────

/// Plan values accepted in the rate-limit request body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Plan {
    Community,
    Pro,
}

impl Default for Plan {
    fn default() -> Self { Plan::Community }
}

#[derive(Debug, Deserialize)]
struct SetRateLimitRequest {
    #[serde(default)]
    plan: Plan,
    /// Override the default hourly limit (optional — uses plan default when absent).
    limit_per_hour: Option<u32>,
}

/// Generate the Postfix main.cf fragment for policyd-rate-limit integration.
///
/// Include this snippet in `/etc/postfix/main.cf` under smtpd_recipient_restrictions
/// and/or smtpd_relay_restrictions.  The check_policy_service line connects to the
/// policyd-rate-limit daemon (typically on 127.0.0.1:10033).
///
/// ```
/// smtpd_recipient_restrictions =
///     permit_mynetworks
///     permit_sasl_authenticated
///     check_policy_service inet:127.0.0.1:10033
///     reject_unauth_destination
/// ```
///
/// policyd-rate-limit reads /etc/postfix/rate_limits as its rate-map file.
/// The map format is:  SENDER_DOMAIN  RATE_PER_HOUR/PERIOD_SECONDS
pub fn postfix_rate_limit_maincf_fragment() -> &'static str {
    r#"# ── JottiCP: policyd-rate-limit integration ──────────────────────────────────
# TODO: install policyd-rate-limit (apt install policyd-rate-limit)
# TODO: configure /etc/policyd-rate-limit.conf with:
#   [limits]
#   default = 500/3600
#   config_file = /etc/postfix/rate_limits
# TODO: start the daemon: systemctl enable --now policyd-rate-limit
#
# Add to /etc/postfix/main.cf:
# smtpd_recipient_restrictions =
#     permit_mynetworks
#     permit_sasl_authenticated
#     check_policy_service inet:127.0.0.1:10033
#     reject_unauth_destination
# ─────────────────────────────────────────────────────────────────────────────
"#
}

/// Path to the Postfix rate_limits hash map.
fn rate_limits_path() -> PathBuf {
    PathBuf::from(
        std::env::var("POSTFIX_RATE_LIMITS")
            .unwrap_or_else(|_| "/etc/postfix/rate_limits".into()),
    )
}

#[instrument(skip(state, body))]
async fn set_domain_rate_limit(
    State(state): State<AppState>,
    Path(domain): Path<String>,
    Json(body): Json<SetRateLimitRequest>,
) -> ApiResult<Json<Value>> {
    let domain = domain.to_lowercase();
    if domain.is_empty() {
        return Err(ApiError::bad_request("domain is required"));
    }

    let limit_per_hour = body.limit_per_hour.unwrap_or_else(|| match body.plan {
        Plan::Pro => 5000,
        Plan::Community => 500,
    });
    // period is always 3600 seconds (one hour)
    let rate_entry = format!("{limit_per_hour}/3600");
    // key in rate_limits file: domain@DOMAIN (policyd-rate-limit SASL user key)
    let map_key = format!("domain@{domain}");

    let _lock = state.write_lock.lock().await;
    let path = rate_limits_path();

    // Replace existing entry or append
    let mut lines = read_lines(&path)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let prefix = format!("{map_key} ");
    let new_line = format!("{map_key} {rate_entry}");
    let mut found = false;
    for line in &mut lines {
        if line.starts_with(&prefix) {
            *line = new_line.clone();
            found = true;
            break;
        }
    }
    if !found {
        lines.push(new_line);
    }

    write_lines(&path, &lines)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    postmap(&path)
        .await
        .map_err(|e| ApiError::internal(format!("postmap rate_limits failed: {e}")))?;

    info!(domain = %domain, limit_per_hour, "rate limit set");
    Ok(Json(json!({
        "domain": domain,
        "limit_per_hour": limit_per_hour,
        "rate_entry": rate_entry,
        "maincf_fragment": postfix_rate_limit_maincf_fragment(),
    })))
}

// ─────────────────────────────────────────────────────────────────────────────
// Step 3.15 — Sieve email filters (raw script CRUD)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct WriteSieveRequest {
    /// Raw Sieve script content.
    script: String,
}

fn sieve_path_for_email(sieve_base: &PathBuf, email: &str) -> Option<PathBuf> {
    // The file layout used in this daemon: /var/mail/sieve/USER@DOMAIN.sieve
    // (flat directory, no per-domain subdirectory — matches managesieve convention)
    if email.contains('/') || email.contains("..") {
        return None;
    }
    Some(sieve_base.join(format!("{email}.sieve")))
}

#[instrument(skip(state))]
async fn get_sieve(
    State(state): State<AppState>,
    Path(email): Path<String>,
) -> ApiResult<Json<Value>> {
    let email = email.to_lowercase();
    let path = sieve_path_for_email(&state.config.sieve_base, &email)
        .ok_or_else(|| ApiError::bad_request("invalid email path"))?;

    match fs::read_to_string(&path).await {
        Ok(content) => Ok(Json(json!({ "email": email, "script": content }))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok(Json(json!({ "email": email, "script": null })))
        }
        Err(e) => Err(ApiError::internal(format!("read sieve failed: {e}"))),
    }
}

#[instrument(skip(state, body))]
async fn put_sieve(
    State(state): State<AppState>,
    Path(email): Path<String>,
    Json(body): Json<WriteSieveRequest>,
) -> ApiResult<Json<Value>> {
    let email = email.to_lowercase();
    let path = sieve_path_for_email(&state.config.sieve_base, &email)
        .ok_or_else(|| ApiError::bad_request("invalid email path"))?;

    if body.script.is_empty() {
        return Err(ApiError::bad_request("script body cannot be empty"));
    }

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| ApiError::internal(format!("create sieve dir: {e}")))?;
    }

    // Write atomically
    let tmp = path.with_extension("tmp");
    let mut f = fs::File::create(&tmp)
        .await
        .map_err(|e| ApiError::internal(format!("create sieve tmp: {e}")))?;
    f.write_all(body.script.as_bytes())
        .await
        .map_err(|e| ApiError::internal(format!("write sieve: {e}")))?;
    f.flush().await.ok();
    drop(f);
    fs::rename(&tmp, &path)
        .await
        .map_err(|e| ApiError::internal(format!("rename sieve: {e}")))?;

    // Compile with sievec to validate
    match run_cmd("sievec", &[path.to_str().unwrap()]).await {
        Ok(_) => {}
        Err(e) => {
            // Remove the invalid script so mail keeps working
            fs::remove_file(&path).await.ok();
            return Err(ApiError::bad_request(format!("sieve compilation failed: {e}")));
        }
    }

    // Activate via doveadm sieve activate
    // doveadm sieve activate -u USER@DOMAIN SCRIPT_NAME
    // The script name is the base filename without .sieve
    let script_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("custom");

    match run_cmd("doveadm", &["sieve", "activate", "-u", &email, script_name]).await {
        Ok(_) => {}
        Err(e) => {
            warn!(email = %email, error = %e, "doveadm sieve activate failed (non-fatal)");
        }
    }

    info!(email = %email, "sieve script written and activated");
    Ok(Json(json!({ "email": email, "status": "activated", "script_name": script_name })))
}

#[instrument(skip(state))]
async fn delete_sieve(
    State(state): State<AppState>,
    Path(email): Path<String>,
) -> ApiResult<StatusCode> {
    let email = email.to_lowercase();
    let path = sieve_path_for_email(&state.config.sieve_base, &email)
        .ok_or_else(|| ApiError::bad_request("invalid email path"))?;

    let compiled = path.with_extension("svbin");

    // Deactivate first
    match run_cmd("doveadm", &["sieve", "deactivate", "-u", &email]).await {
        Ok(_) => {}
        Err(e) => {
            warn!(email = %email, error = %e, "doveadm sieve deactivate failed (non-fatal)");
        }
    }

    for f in [&path, &compiled] {
        match fs::remove_file(f).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(ApiError::internal(format!("remove sieve file: {e}")));
            }
        }
    }

    info!(email = %email, "sieve script deleted");
    Ok(StatusCode::NO_CONTENT)
}

// ─────────────────────────────────────────────────────────────────────────────
// Step 3.12 — IP binding per domain (for reseller dedicated IPs)
// ─────────────────────────────────────────────────────────────────────────────

/// Path to Postfix sender_transport map — used to bind a domain to a specific
/// outbound IP via smtp_bind:<IP> transport.
fn sender_transport_path() -> PathBuf {
    PathBuf::from(
        std::env::var("POSTFIX_SENDER_TRANSPORT")
            .unwrap_or_else(|_| "/etc/postfix/sender_transport".into()),
    )
}

/// Path to Postfix smtp_bind_map — maps transport name to inet_interfaces.
fn smtp_bind_map_path() -> PathBuf {
    PathBuf::from(
        std::env::var("POSTFIX_SMTP_BIND_MAP")
            .unwrap_or_else(|_| "/etc/postfix/smtp_bind_map".into()),
    )
}

#[derive(Debug, Deserialize)]
struct IpBindRequest {
    domain:     String,
    ip_address: String,
}

/// POST /internal/email/ip-bind
///
/// Writes the following to Postfix config files:
///   /etc/postfix/sender_transport:  @domain   smtp_bind:IP_ADDRESS
///   /etc/postfix/smtp_bind_map:     IP_ADDRESS inet_interfaces=IP_ADDRESS
/// Then runs postmap on sender_transport and reloads Postfix.
#[instrument(skip(state, body))]
async fn ip_bind(
    State(state): State<AppState>,
    Json(body): Json<IpBindRequest>,
) -> ApiResult<Json<Value>> {
    let domain = body.domain.to_lowercase();
    if domain.is_empty() {
        return Err(ApiError::bad_request("domain is required"));
    }
    // Basic IP validation — must parse as an IP address
    let ip = body.ip_address.trim().to_string();
    if ip.is_empty() || ip.parse::<std::net::IpAddr>().is_err() {
        return Err(ApiError::bad_request(format!("invalid ip_address: {ip}")));
    }

    let _lock = state.write_lock.lock().await;

    // sender_transport: "@domain  smtp_bind:IP"
    let transport_key = format!("@{domain}");
    let transport_entry = format!("{transport_key}   smtp_bind:{ip}");
    upsert_line(&sender_transport_path(), &format!("{transport_key} "), &transport_entry)
        .await
        .map_err(|e| ApiError::internal(format!("sender_transport write failed: {e}")))?;

    postmap(&sender_transport_path())
        .await
        .map_err(|e| ApiError::internal(format!("postmap sender_transport failed: {e}")))?;

    // smtp_bind_map: "IP  inet_interfaces=IP"
    let bind_entry = format!("{ip}   inet_interfaces={ip}");
    upsert_line(&smtp_bind_map_path(), &format!("{ip} "), &bind_entry)
        .await
        .map_err(|e| ApiError::internal(format!("smtp_bind_map write failed: {e}")))?;

    postfix_reload()
        .await
        .map_err(|e| ApiError::internal(format!("postfix reload failed: {e}")))?;

    info!(domain = %domain, ip = %ip, "IP bind configured");
    Ok(Json(json!({
        "domain":     domain,
        "ip_address": ip,
        "status":     "configured",
    })))
}

/// DELETE /internal/email/ip-bind/:domain
///
/// Removes the smtp_bind_address config for the given domain from
/// sender_transport, then reloads Postfix.
#[instrument(skip(state))]
async fn ip_unbind(
    State(state): State<AppState>,
    Path(domain): Path<String>,
) -> ApiResult<StatusCode> {
    let domain = domain.to_lowercase();
    let _lock = state.write_lock.lock().await;

    let transport_key = format!("@{domain}");
    let removed = remove_lines_matching(&sender_transport_path(), &format!("{transport_key} "))
        .await
        .map_err(|e| ApiError::internal(format!("sender_transport remove failed: {e}")))?;

    if removed == 0 {
        return Err(ApiError::not_found(format!(
            "no IP binding found for domain '{domain}'"
        )));
    }

    postmap(&sender_transport_path())
        .await
        .map_err(|e| ApiError::internal(format!("postmap sender_transport failed: {e}")))?;

    postfix_reload()
        .await
        .map_err(|e| ApiError::internal(format!("postfix reload failed: {e}")))?;

    info!(domain = %domain, "IP bind removed");
    Ok(StatusCode::NO_CONTENT)
}

// ─────────────────────────────────────────────────────────────────────────────
// Step 3.16 — Smarthost / outbound relay per domain
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct SetSmarthostRequest {
    host:     String,
    port:     u16,
    username: Option<String>,
    password: Option<String>,
    /// "tls" | "starttls" | "none"
    #[serde(default = "default_tls_mode")]
    tls:      String,
}

fn default_tls_mode() -> String { "tls".into() }

fn transport_maps_path() -> PathBuf {
    PathBuf::from(
        std::env::var("POSTFIX_TRANSPORT_MAPS")
            .unwrap_or_else(|_| "/etc/postfix/transport_maps".into()),
    )
}

fn sasl_passwd_path() -> PathBuf {
    PathBuf::from(
        std::env::var("POSTFIX_SASL_PASSWD")
            .unwrap_or_else(|_| "/etc/postfix/sasl_passwd".into()),
    )
}

fn postfix_maincf_path() -> PathBuf {
    PathBuf::from(
        std::env::var("POSTFIX_MAIN_CF")
            .unwrap_or_else(|_| "/etc/postfix/main.cf".into()),
    )
}

/// Ensure the sasl-auth directives are present in main.cf.
/// Idempotent — only appends if the key is absent.
async fn ensure_sasl_in_maincf(tls: &str) -> Result<()> {
    let path = postfix_maincf_path();
    let content = match fs::read_to_string(&path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).context("read main.cf"),
    };

    let mut lines: Vec<String> = content.lines().map(String::from).collect();
    let mut changed = false;

    let required: &[(&str, &str)] = &[
        ("smtp_sasl_auth_enable", "yes"),
        ("smtp_sasl_password_maps", "hash:/etc/postfix/sasl_passwd"),
        ("smtp_sasl_security_options", "noanonymous"),
    ];

    for (key, val) in required {
        if !lines.iter().any(|l| l.starts_with(key)) {
            lines.push(format!("{key} = {val}"));
            changed = true;
        }
    }

    // TLS security level
    let tls_level = match tls {
        "none" => "may",
        "starttls" => "encrypt",
        _ => "encrypt",   // default: enforce TLS
    };
    let tls_key = "smtp_tls_security_level";
    if !lines.iter().any(|l| l.starts_with(tls_key)) {
        lines.push(format!("{tls_key} = {tls_level}"));
        changed = true;
    }

    if changed {
        let tmp = path.with_extension("tmp");
        let mut f = tokio::fs::File::create(&tmp).await.context("create main.cf tmp")?;
        for l in &lines {
            f.write_all(l.as_bytes()).await?;
            f.write_all(b"\n").await?;
        }
        f.flush().await?;
        drop(f);
        tokio::fs::rename(&tmp, &path).await.context("rename main.cf")?;
    }

    Ok(())
}

#[instrument(skip(state, body))]
async fn set_smarthost(
    State(state): State<AppState>,
    Path(domain): Path<String>,
    Json(body): Json<SetSmarthostRequest>,
) -> ApiResult<Json<Value>> {
    let domain = domain.to_lowercase();
    if domain.is_empty() || body.host.is_empty() {
        return Err(ApiError::bad_request("domain and host are required"));
    }
    if body.port == 0 {
        return Err(ApiError::bad_request("port must be > 0"));
    }

    let _lock = state.write_lock.lock().await;

    // transport_maps: "DOMAIN smtp:[HOST]:PORT"
    let transport_key = domain.clone();
    let relay_target = format!("smtp:[{}]:{}", body.host, body.port);
    let transport_entry = format!("{transport_key} {relay_target}");

    upsert_line(&transport_maps_path(), &format!("{transport_key} "), &transport_entry)
        .await
        .map_err(|e| ApiError::internal(format!("transport_maps write: {e}")))?;

    postmap(&transport_maps_path())
        .await
        .map_err(|e| ApiError::internal(format!("postmap transport_maps: {e}")))?;

    // sasl_passwd: "[HOST]:PORT USERNAME:PASSWORD"
    if let (Some(user), Some(pass)) = (&body.username, &body.password) {
        let sasl_key = format!("[{}]:{}", body.host, body.port);
        let sasl_entry = format!("{sasl_key} {user}:{pass}");

        upsert_line(&sasl_passwd_path(), &format!("{sasl_key} "), &sasl_entry)
            .await
            .map_err(|e| ApiError::internal(format!("sasl_passwd write: {e}")))?;

        postmap(&sasl_passwd_path())
            .await
            .map_err(|e| ApiError::internal(format!("postmap sasl_passwd: {e}")))?;

        // Ensure SASL enabled in main.cf
        ensure_sasl_in_maincf(&body.tls)
            .await
            .map_err(|e| ApiError::internal(format!("main.cf update: {e}")))?;
    }

    postfix_reload()
        .await
        .map_err(|e| ApiError::internal(format!("postfix reload: {e}")))?;

    info!(domain = %domain, host = %body.host, port = body.port, "smarthost set");
    Ok(Json(json!({
        "domain": domain,
        "relay": relay_target,
        "status": "configured",
    })))
}

#[instrument(skip(state))]
async fn delete_smarthost(
    State(state): State<AppState>,
    Path(domain): Path<String>,
) -> ApiResult<StatusCode> {
    let domain = domain.to_lowercase();
    let _lock = state.write_lock.lock().await;

    remove_lines_matching(&transport_maps_path(), &format!("{domain} "))
        .await
        .map_err(|e| ApiError::internal(format!("transport_maps remove: {e}")))?;

    postmap(&transport_maps_path())
        .await
        .map_err(|e| ApiError::internal(format!("postmap transport_maps: {e}")))?;

    // Note: we do NOT remove from sasl_passwd because the [HOST]:PORT key is not
    // domain-keyed; removing it could affect other domains sharing the same relay.
    // TODO: track relay→domain mapping in a side-file to enable safe cleanup.

    postfix_reload()
        .await
        .map_err(|e| ApiError::internal(format!("postfix reload: {e}")))?;

    info!(domain = %domain, "smarthost removed");
    Ok(StatusCode::NO_CONTENT)
}

/// Upsert a line: replace the first line starting with `prefix` or append.
async fn upsert_line(path: &PathBuf, prefix: &str, new_line: &str) -> Result<()> {
    let mut lines = read_lines(path).await?;
    let mut found = false;
    for line in &mut lines {
        if line.starts_with(prefix) {
            *line = new_line.to_string();
            found = true;
            break;
        }
    }
    if !found {
        lines.push(new_line.to_string());
    }
    write_lines(path, &lines).await
}

// ─────────────────────────────────────────────────────────────────────────────
// Step 3.17 — Mail queue management (per-message flush / delete)
// ─────────────────────────────────────────────────────────────────────────────

#[instrument(skip(_state))]
async fn flush_queue_message(
    State(_state): State<AppState>,
    Path(queue_id): Path<String>,
) -> ApiResult<Json<Value>> {
    if !queue_id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(ApiError::bad_request("invalid queue_id"));
    }
    run_cmd("postqueue", &["-i", &queue_id])
        .await
        .map_err(|e| ApiError::internal(format!("postqueue -i failed: {e}")))?;
    info!(queue_id = %queue_id, "queue message flushed");
    Ok(Json(json!({ "queue_id": queue_id, "status": "flushed" })))
}

#[instrument(skip(_state))]
async fn flush_all_queue(State(_state): State<AppState>) -> ApiResult<Json<Value>> {
    run_cmd("postqueue", &["-f"])
        .await
        .map_err(|e| ApiError::internal(format!("postqueue -f failed: {e}")))?;
    info!("full mail queue flush requested");
    Ok(Json(json!({ "status": "flush_requested" })))
}

// ─────────────────────────────────────────────────────────────────────────────
// Handlers — DKIM
// ─────────────────────────────────────────────────────────────────────────────

#[instrument(skip(state))]
async fn generate_dkim(
    State(state): State<AppState>,
    Path(domain): Path<String>,
) -> ApiResult<Json<Value>> {
    let domain = domain.to_lowercase();

    fs::create_dir_all(&state.config.dkim_key_dir)
        .await
        .map_err(|e| ApiError::internal(format!("failed to create DKIM key dir: {e}")))?;

    let key_file = state
        .config
        .dkim_key_dir
        .join(format!("{domain}.mail.key"));

    let output = Command::new("rspamadm")
        .args([
            "dkim_keygen",
            "-s",
            "mail",
            "-d",
            &domain,
            "-k",
            key_file.to_str().unwrap(),
        ])
        .output()
        .await
        .map_err(|e| ApiError::internal(format!("failed to run rspamadm: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ApiError::internal(format!("rspamadm failed: {stderr}")));
    }

    // Secure the private key — rspamadm may write it 0644 by default.
    // It must be 0600 so only the rspamd process (running as root or dedicated
    // user) can read it.
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&key_file, std::fs::Permissions::from_mode(0o600)) {
            return Err(ApiError::internal(format!(
                "failed to set DKIM key permissions to 0600: {e}"
            )));
        }
    }

    // rspamadm prints the DNS TXT record value to stdout
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();

    // Extract the p= value from the output — rspamadm prints the full TXT record
    let dns_value = stdout
        .lines()
        .find(|l| l.contains("v=DKIM1"))
        .map(|l| l.trim().to_string())
        .unwrap_or_else(|| stdout.trim().to_string());

    let dns_name = format!("mail._domainkey.{domain}");

    info!(domain = %domain, "DKIM keypair generated");
    Ok(Json(json!({
        "dns_name": dns_name,
        "dns_value": dns_value,
        "key_file": key_file.display().to_string(),
    })))
}

// ─────────────────────────────────────────────────────────────────────────────
// Handlers — Mail Queue
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct QueueEntry {
    id: String,
    size: u64,
    arrival_time: String,
    sender: String,
    recipients: Vec<String>,
    reason: Option<String>,
}

/// Parse `mailq` output into structured entries.
/// mailq format:
/// -Queue ID- --Size-- ----Arrival Time---- -Sender/Recipient-------
/// ABC123     1234     Thu May  1 10:00:00  sender@example.com
///                                          recipient@example.com
///
/// Uses a regex to handle single-digit day-of-month dates, which produce
/// variable-width columns and break a simple splitn-based approach.
fn parse_mailq(output: &str) -> Vec<QueueEntry> {
    // Captures: (queue_id, size, date, sender)
    // Date format: "DDD DDD  D HH:MM:SS" or "DDD DDD DD HH:MM:SS"
    let entry_re = Regex::new(
        r"^([0-9A-F]+)\s+(\S+)\s+(\w{3}\s+\w{3}\s+\d+\s+\d{2}:\d{2}:\d{2})\s+(\S+)"
    ).unwrap();

    let mut entries: Vec<QueueEntry> = Vec::new();
    let mut current: Option<QueueEntry> = None;

    for line in output.lines() {
        // Skip header / empty
        if line.starts_with('-') || line.is_empty() || line.starts_with("Mail queue is empty") {
            continue;
        }

        // New entry: starts with alphanum queue ID (not a space)
        if line.chars().next().map(|c| c.is_ascii_alphanumeric()).unwrap_or(false)
            && !line.starts_with(' ')
        {
            if let Some(entry) = current.take() {
                entries.push(entry);
            }

            if let Some(caps) = entry_re.captures(line) {
                let queue_id = caps[1].trim_start_matches('!').to_string();
                let size: u64 = caps[2].parse().unwrap_or(0);
                let arrival_time = caps[3].to_string();
                let sender = caps[4].to_string();
                current = Some(QueueEntry {
                    id: queue_id,
                    size,
                    arrival_time,
                    sender,
                    recipients: vec![],
                    reason: None,
                });
            }
            continue;
        }

        // Continuation line: recipient or reason
        if let Some(ref mut entry) = current {
            let trimmed = line.trim().to_string();
            if trimmed.contains('@') {
                entry.recipients.push(trimmed);
            } else if !trimmed.is_empty() {
                entry.reason = Some(trimmed);
            }
        }
    }

    if let Some(entry) = current {
        entries.push(entry);
    }

    entries
}

#[instrument(skip(_state))]
async fn get_queue(State(_state): State<AppState>) -> ApiResult<Json<Value>> {
    let output = run_cmd("mailq", &[])
        .await
        .map_err(|e| ApiError::internal(format!("mailq failed: {e}")))?;

    let entries = parse_mailq(&output);
    info!(count = entries.len(), "mail queue fetched");
    Ok(Json(json!({ "queue": entries, "count": entries.len() })))
}

#[instrument(skip(_state))]
async fn flush_queue(State(_state): State<AppState>) -> ApiResult<Json<Value>> {
    run_cmd("postfix", &["flush"])
        .await
        .map_err(|e| ApiError::internal(format!("postfix flush failed: {e}")))?;
    info!("mail queue flushed");
    Ok(Json(json!({ "status": "flushed" })))
}

#[instrument(skip(_state))]
async fn delete_queue_message(
    State(_state): State<AppState>,
    Path(queue_id): Path<String>,
) -> ApiResult<StatusCode> {
    // Validate queue_id — only alphanumeric to prevent injection
    if !queue_id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(ApiError::bad_request("invalid queue_id"));
    }

    run_cmd("postsuper", &["-d", &queue_id])
        .await
        .map_err(|e| ApiError::internal(format!("postsuper -d failed: {e}")))?;

    info!(queue_id = %queue_id, "mail queue message deleted");
    Ok(StatusCode::NO_CONTENT)
}

// ─────────────────────────────────────────────────────────────────────────────
// Router
// ─────────────────────────────────────────────────────────────────────────────

fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        // Domains
        .route("/internal/email/domains", post(add_domain))
        .route("/domains", post(add_domain))
        .route("/internal/email/domains/{domain}", delete(delete_domain))
        .route("/domains/{domain}", delete(delete_domain))
        // Accounts
        .route("/internal/email/accounts", post(create_account))
        .route("/accounts", post(create_account))
        .route("/internal/email/accounts/{email}", delete(delete_account))
        .route("/accounts/{email}", delete(delete_account))
        .route("/internal/email/accounts/{email}/password", post(change_password))
        .route("/accounts/{email}/password", post(change_password))
        // Forwarders
        .route("/internal/email/forwarders", post(create_forwarder))
        .route("/forwarders", post(create_forwarder))
        .route("/internal/email/forwarders/{from_address}", delete(delete_forwarder))
        .route("/forwarders/{from_address}", delete(delete_forwarder))
        // Autoresponders
        .route("/internal/email/autoresponders", post(create_autoresponder))
        .route("/autoresponders", post(create_autoresponder))
        .route("/internal/email/autoresponders/{email}", delete(delete_autoresponder))
        .route("/autoresponders/{email}", delete(delete_autoresponder))
        // DKIM
        .route("/internal/email/domains/{domain}/dkim", post(generate_dkim))
        .route("/domains/{domain}/dkim", post(generate_dkim))
        // Step 3.9 — Rate limits
        .route("/internal/email/domains/{domain}/rate-limit", post(set_domain_rate_limit))
        // Step 3.12 — Dedicated IP binding per domain
        .route("/internal/email/ip-bind", post(ip_bind))
        .route("/internal/email/ip-bind/{domain}", delete(ip_unbind))
        // Step 3.15 — Sieve filters
        .route("/internal/email/mailboxes/{email}/sieve", get(get_sieve).put(put_sieve).delete(delete_sieve))
        // Step 3.16 — Smarthost
        .route("/internal/email/domains/{domain}/smarthost", post(set_smarthost).delete(delete_smarthost))
        // Step 3.17 — Queue management (per-message + bulk)
        .route("/internal/email/queue", get(get_queue))
        .route("/queue", get(get_queue))
        .route("/internal/email/queue/flush-all", post(flush_all_queue))
        .route("/queue/flush", post(flush_queue))
        .route("/internal/email/queue/{queue_id}/flush", post(flush_queue_message))
        .route("/internal/email/queue/{queue_id}", delete(delete_queue_message))
        .route("/queue/{queue_id}", delete(delete_queue_message))
        .with_state(state)
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "jotti-mail" }))
}

// ─────────────────────────────────────────────────────────────────────────────
// Main
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("jotti_mail=info".parse().unwrap()),
        )
        .init();

    let config = Config::from_env().context("failed to load configuration")?;
    let listen_addr = config.listen_addr;

    info!(listen = %listen_addr, "jotti-mail starting");

    let state = AppState::new(config);
    let router = build_router(state);

    let listener = tokio::net::TcpListener::bind(listen_addr)
        .await
        .context("failed to bind listener")?;

    info!(addr = %listen_addr, "jotti-mail listening");

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;

    info!("jotti-mail stopped");
    Ok(())
}

async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut sigterm = signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
    let mut sigint = signal(SignalKind::interrupt()).expect("failed to install SIGINT handler");
    tokio::select! {
        _ = sigterm.recv() => { info!("received SIGTERM, shutting down"); }
        _ = sigint.recv()  => { info!("received SIGINT, shutting down"); }
    }
}
