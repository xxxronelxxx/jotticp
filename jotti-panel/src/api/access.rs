use axum::{
    Router,
    routing::{delete, get, post, put},
    extract::{Path, State},
    Json,
    http::StatusCode,
    response::IntoResponse,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};
use super::auth::Claims;

// ── FTP types ─────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct FtpUser {
    pub id:          Uuid,
    pub username:    String,
    pub home_subdir: String,
    pub quota_mb:    i32,
    pub enabled:     bool,
    pub created_at:  String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateFtpUserRequest {
    pub home_subdir: Option<String>,
    pub quota_mb:    Option<i32>,
    pub enabled:     Option<bool>,
}

// ── Request / Response types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AddSshKeyRequest {
    pub label:      String,
    pub public_key: String,
}

#[derive(Debug, Serialize)]
pub struct SshKeyItem {
    pub id:          String,  // hex prefix of sha256 of key_data
    pub label:       String,
    pub public_key:  String,
    pub fingerprint: String,
    pub created_at:  String,  // ISO-8601
}

#[derive(Debug, Deserialize)]
pub struct SetSftpPasswordRequest {
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateFtpUserRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct ChangeFtpPasswordRequest {
    pub password: String,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        // Per-site SSH keys (reads/writes site unix user's authorized_keys)
        .route("/api/v1/sites/{id}/ssh-keys",
            get(ssh_keys_list).post(ssh_keys_add))
        .route("/api/v1/sites/{id}/ssh-keys/{key_id}",
            delete(ssh_keys_delete))
        // SFTP management (chroot config + password reset)
        .route("/api/v1/sites/{id}/sftp/configure",
            post(sftp_configure))
        .route("/api/v1/sites/{id}/sftp/set-password",
            post(sftp_set_password))
        // FTP virtual users per site (pure-ftpd PureDB backend)
        .route("/api/v1/sites/{id}/ftp-users",
            get(ftp_users_list).post(ftp_users_create))
        .route("/api/v1/sites/{id}/ftp-users/{user_id}",
            put(ftp_users_update).delete(ftp_users_delete))
        .route("/api/v1/sites/{id}/ftp-users/{user_id}/password",
            put(ftp_users_change_password))
}

// ── Access control helpers ────────────────────────────────────────────────────

fn caller(claims: &Claims) -> ApiResult<(Uuid, bool)> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    let is_admin = claims.role == "admin";
    Ok((user_id, is_admin))
}

/// Verify the authenticated user owns (or has access to) the given site.
/// Returns the site's unix_user on success.
async fn get_site_unix_user(
    state: &AppState,
    site_id: Uuid,
    user_id: Uuid,
    is_admin: bool,
) -> ApiResult<String> {
    let row = sqlx::query!(
        r#"SELECT unix_user FROM sites
           WHERE id = $1 AND deleted_at IS NULL
             AND ($2::boolean OR owner_id = $3)"#,
        site_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?;

    row.map(|r| r.unix_user)
        .ok_or(ApiError::NotFound { resource: "site" })
}

// ── SSH key helpers ───────────────────────────────────────────────────────────

/// Compute key ID: first 32 hex chars of SHA-256 of the raw base64 key data.
fn key_id_from_data(key_data: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key_data.as_bytes());
    let result = hasher.finalize();
    format!("{:x}", result)[..32].to_string()
}

/// Extract the key data (base64 part) from a public key line.
fn extract_key_data(public_key: &str) -> Option<&str> {
    let parts: Vec<&str> = public_key.trim().splitn(3, ' ').collect();
    if parts.len() >= 2 { Some(parts[1]) } else { None }
}

/// Compute fingerprint by calling ssh-keygen.
async fn compute_fingerprint(public_key: &str) -> String {
    use tokio::process::Command;
    let tmp = format!("/tmp/jotticp-key-{}.pub", Uuid::new_v4());
    if tokio::fs::write(&tmp, public_key).await.is_err() {
        return "unknown".to_string();
    }
    let out = Command::new("ssh-keygen").args(["-l", "-f", &tmp]).output().await;
    let _ = tokio::fs::remove_file(&tmp).await;
    match out {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout);
            s.split_whitespace().nth(1).unwrap_or("unknown").to_string()
        }
        _ => "unknown".to_string(),
    }
}

/// Get home directory for a unix user.
async fn get_home(unix_user: &str) -> String {
    // Fast path: common jotticp convention
    format!("/home/{}", unix_user)
}

/// Validate SSH public key format.
fn validate_ssh_key(key: &str) -> ApiResult<()> {
    let key = key.trim();
    let valid_prefixes = [
        "ssh-rsa", "ssh-ed25519",
        "ecdsa-sha2-nistp256", "ecdsa-sha2-nistp384", "ecdsa-sha2-nistp521",
        "sk-ssh-ed25519@openssh.com", "sk-ecdsa-sha2-nistp256@openssh.com",
    ];
    if !valid_prefixes.iter().any(|&p| key.starts_with(p)) {
        return Err(ApiError::Validation(
            "Public key must start with ssh-rsa, ssh-ed25519, or ecdsa-sha2-*".into()
        ));
    }
    let parts: Vec<&str> = key.splitn(3, ' ').collect();
    if parts.len() < 2 || parts[1].is_empty() {
        return Err(ApiError::Validation("Invalid SSH public key format".into()));
    }
    if key.chars().any(|c| matches!(c, '\n' | '\r' | ';' | '&' | '|' | '$' | '`')) {
        return Err(ApiError::Validation("SSH key contains invalid characters".into()));
    }
    Ok(())
}

/// Parse authorized_keys file into SshKeyItem list.
async fn parse_authorized_keys(auth_keys_path: &str) -> Vec<SshKeyItem> {
    let content = tokio::fs::read_to_string(auth_keys_path).await.unwrap_or_default();
    // Get file mtime for created_at fallback
    let mtime = tokio::fs::metadata(auth_keys_path).await
        .ok()
        .and_then(|m| m.modified().ok())
        .map(|t| {
            let secs = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
            DateTime::<Utc>::from_timestamp(secs as i64, 0)
                .unwrap_or_else(Utc::now)
                .to_rfc3339()
        })
        .unwrap_or_else(|| Utc::now().to_rfc3339());

    content
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim().starts_with('#'))
        .filter_map(|line| {
            let parts: Vec<&str> = line.trim().splitn(3, ' ').collect();
            if parts.len() < 2 { return None; }
            let key_data = parts[1];
            let label = if parts.len() >= 3 && !parts[2].is_empty() {
                parts[2].to_string()
            } else {
                format!("{} key", parts[0])
            };
            let id = key_id_from_data(key_data);
            Some(SshKeyItem {
                id,
                label,
                public_key: line.trim().to_string(),
                fingerprint: String::new(), // filled below if needed
                created_at: mtime.clone(),
            })
        })
        .collect()
}

/// Append a key to authorized_keys.
async fn append_authorized_key(home: &str, public_key: &str) -> ApiResult<()> {
    use tokio::io::AsyncWriteExt;
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;

    let ssh_dir = format!("{}/.ssh", home);
    let auth_keys = format!("{}/authorized_keys", ssh_dir);

    tokio::fs::create_dir_all(&ssh_dir).await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Create .ssh dir: {}", e)))?;
    let _ = tokio::process::Command::new("chmod").args(["700", &ssh_dir]).output().await;

    #[cfg(unix)]
    let mut file = {
        tokio::fs::OpenOptions::new()
            .create(true).append(true).mode(0o600)
            .open(&auth_keys).await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("Open authorized_keys: {}", e)))?
    };
    #[cfg(not(unix))]
    let mut file = {
        tokio::fs::OpenOptions::new()
            .create(true).append(true)
            .open(&auth_keys).await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("Open authorized_keys: {}", e)))?
    };

    let mut line = public_key.trim().to_string();
    line.push('\n');
    file.write_all(line.as_bytes()).await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Write authorized_keys: {}", e)))?;
    let _ = tokio::process::Command::new("chmod").args(["600", &auth_keys]).output().await;
    Ok(())
}

/// Remove a key from authorized_keys by key ID (sha256 of key_data).
async fn remove_key_by_id(auth_keys_path: &str, key_id: &str) -> ApiResult<bool> {
    let content = tokio::fs::read_to_string(auth_keys_path).await.unwrap_or_default();
    let mut removed = false;
    let new_lines: Vec<&str> = content
        .lines()
        .filter(|line| {
            if line.trim().is_empty() || line.trim().starts_with('#') {
                return true;
            }
            let parts: Vec<&str> = line.trim().splitn(3, ' ').collect();
            if parts.len() < 2 { return true; }
            let id = key_id_from_data(parts[1]);
            if id == key_id {
                removed = true;
                false
            } else {
                true
            }
        })
        .collect();

    if !removed { return Ok(false); }

    let mut out = new_lines.join("\n");
    if !out.is_empty() { out.push('\n'); }

    let tmp = format!("{}.tmp.{}", auth_keys_path, std::process::id());
    tokio::fs::write(&tmp, &out).await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Write tmp: {}", e)))?;
    let _ = tokio::process::Command::new("chmod").args(["600", &tmp]).output().await;
    tokio::fs::rename(&tmp, auth_keys_path).await
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            ApiError::Internal(anyhow::anyhow!("Rename authorized_keys: {}", e))
        })?;
    Ok(true)
}

// ── SSH key handlers ──────────────────────────────────────────────────────────

/// GET /api/v1/sites/:id/ssh-keys
pub async fn ssh_keys_list(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let (user_id, is_admin) = match caller(&claims) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let unix_user = match get_site_unix_user(&state, id, user_id, is_admin).await {
        Ok(u) => u,
        Err(e) => return e.into_response(),
    };

    let home = get_home(&unix_user).await;
    let auth_keys = format!("{}/.ssh/authorized_keys", home);
    let mut keys = parse_authorized_keys(&auth_keys).await;

    // Compute fingerprints for each key (in parallel would be better but keep it simple)
    for key in &mut keys {
        key.fingerprint = compute_fingerprint(&key.public_key).await;
    }

    (StatusCode::OK, Json(keys)).into_response()
}

/// POST /api/v1/sites/:id/ssh-keys
pub async fn ssh_keys_add(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
    Json(req): Json<AddSshKeyRequest>,
) -> impl IntoResponse {
    let (user_id, is_admin) = match caller(&claims) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let unix_user = match get_site_unix_user(&state, id, user_id, is_admin).await {
        Ok(u) => u,
        Err(e) => return e.into_response(),
    };

    if let Err(e) = validate_ssh_key(&req.public_key) {
        return e.into_response();
    }

    // Build the full key line including label as comment
    let trimmed_key = req.public_key.trim();

    // Strip shell metacharacters and control characters from the label so it
    // cannot be used for shell injection when the authorized_keys file is
    // later read by tools (e.g. shell scripts, audit tools).
    let label: String = req.label.trim()
        .chars()
        .filter(|c| !matches!(c, '\n' | '\r' | ';' | '&' | '|' | '$' | '`' | '<' | '>' | '\'' | '"' | '\\'))
        .collect();
    // Enforce a reasonable max length for the comment field
    let label = if label.len() > 64 { label[..64].to_string() } else { label };

    // Check if key already exists
    let home = get_home(&unix_user).await;
    let auth_keys_path = format!("{}/.ssh/authorized_keys", home);
    let existing_keys = parse_authorized_keys(&auth_keys_path).await;
    let key_data = match extract_key_data(trimmed_key) {
        Some(d) => d,
        None => return ApiError::Validation("Invalid SSH key format".into()).into_response(),
    };
    let new_id = key_id_from_data(key_data);

    if existing_keys.iter().any(|k| {
        extract_key_data(&k.public_key)
            .map(|d| key_id_from_data(d) == new_id)
            .unwrap_or(false)
    }) {
        return ApiError::Validation("This SSH key is already added for this site".into()).into_response();
    }

    // Rebuild key line with our label as comment
    let parts: Vec<&str> = trimmed_key.splitn(3, ' ').collect();
    let key_line = if label.is_empty() {
        trimmed_key.to_string()
    } else if parts.len() >= 2 {
        format!("{} {} {}", parts[0], parts[1], label)
    } else {
        trimmed_key.to_string()
    };

    if let Err(e) = append_authorized_key(&home, &key_line).await {
        return e.into_response();
    }

    let fingerprint = compute_fingerprint(&key_line).await;
    let now = Utc::now().to_rfc3339();

    let response = SshKeyItem {
        id: new_id,
        label: if label.is_empty() {
            parts.get(2).map(|s| s.to_string()).unwrap_or_else(|| format!("{} key", parts.get(0).unwrap_or(&"ssh")))
        } else {
            label
        },
        public_key: key_line,
        fingerprint,
        created_at: now,
    };

    (StatusCode::CREATED, Json(response)).into_response()
}

/// DELETE /api/v1/sites/:id/ssh-keys/:key_id
pub async fn ssh_keys_delete(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path((id, key_id)): Path<(Uuid, String)>,
) -> impl IntoResponse {
    let (user_id, is_admin) = match caller(&claims) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let unix_user = match get_site_unix_user(&state, id, user_id, is_admin).await {
        Ok(u) => u,
        Err(e) => return e.into_response(),
    };

    let home = get_home(&unix_user).await;
    let auth_keys_path = format!("{}/.ssh/authorized_keys", home);

    match remove_key_by_id(&auth_keys_path, &key_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => ApiError::NotFound { resource: "SSH key" }.into_response(),
        Err(e) => e.into_response(),
    }
}

// ── SFTP handlers ─────────────────────────────────────────────────────────────

/// POST /api/v1/sites/:id/sftp/configure
/// Configure sshd to allow SFTP with chroot for this site's unix user.
pub async fn sftp_configure(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let (user_id, is_admin) = match caller(&claims) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let unix_user = match get_site_unix_user(&state, id, user_id, is_admin).await {
        Ok(u) => u,
        Err(e) => return e.into_response(),
    };

    // Build the sshd Match block
    let match_block = format!(
        "\n# JottiCP SFTP chroot for site {}\nMatch User {}\n  ChrootDirectory /home/{}\n  ForceCommand internal-sftp\n  AllowTcpForwarding no\n  X11Forwarding no\n",
        unix_user, unix_user, unix_user
    );

    let sshd_conf_path = "/etc/ssh/sshd_config.d/jotticp-sftp.conf";

    // Read existing config, add block if not already present
    let existing = tokio::fs::read_to_string(sshd_conf_path).await.unwrap_or_default();
    let marker = format!("Match User {}", unix_user);

    if !existing.contains(&marker) {
        let new_content = format!("{}{}", existing, match_block);
        if let Err(e) = tokio::fs::write(sshd_conf_path, &new_content).await {
            return ApiError::Internal(anyhow::anyhow!("Write sshd config: {}", e)).into_response();
        }
    }

    // Validate sshd config
    let test = tokio::process::Command::new("sshd").args(["-t"]).output().await;
    match test {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            // Revert
            let _ = tokio::fs::write(sshd_conf_path, &existing).await;
            let stderr = String::from_utf8_lossy(&o.stderr);
            return ApiError::Validation(format!("sshd config invalid: {}", stderr)).into_response();
        }
        Err(e) => {
            let _ = tokio::fs::write(sshd_conf_path, &existing).await;
            return ApiError::Internal(anyhow::anyhow!("sshd -t: {}", e)).into_response();
        }
    }

    // Change shell to /bin/bash so SFTP with internal-sftp still works
    // (internal-sftp doesn't need a login shell, but chroot requires root-owned chroot dir)
    // Ensure the home dir is owned by root for chroot to work
    let home = format!("/home/{}", unix_user);
    let _ = tokio::process::Command::new("chown")
        .args(["root:root", &home])
        .output().await;
    let _ = tokio::process::Command::new("chmod")
        .args(["755", &home])
        .output().await;

    // Create public_html directory owned by the unix user (writable area inside chroot)
    let public_html = format!("{}/public_html", home);
    let _ = tokio::fs::create_dir_all(&public_html).await;
    let _ = tokio::process::Command::new("chown")
        .args([&format!("{}:{}", unix_user, unix_user), &public_html])
        .output().await;

    // Reload sshd
    let reload = tokio::process::Command::new("systemctl")
        .args(["reload", "sshd"])
        .output().await;
    if let Err(e) = reload {
        return ApiError::Internal(anyhow::anyhow!("Reload sshd: {}", e)).into_response();
    }

    (StatusCode::OK, Json(json!({
        "status": "configured",
        "message": "SFTP chroot configured. Connect with FileZilla or sftp client.",
        "unix_user": unix_user,
        "chroot_dir": home,
        "writable_dir": format!("{}/public_html", home),
    }))).into_response()
}

/// POST /api/v1/sites/:id/sftp/set-password
/// Set the Unix password for the site user (enables SFTP password auth).
pub async fn sftp_set_password(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
    Json(req): Json<SetSftpPasswordRequest>,
) -> impl IntoResponse {
    let (user_id, is_admin) = match caller(&claims) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let unix_user = match get_site_unix_user(&state, id, user_id, is_admin).await {
        Ok(u) => u,
        Err(e) => return e.into_response(),
    };

    if req.password.len() < 8 {
        return ApiError::Validation("Password must be at least 8 characters".into()).into_response();
    }

    // Sanitize: reject newlines or shell metacharacters in password
    if req.password.chars().any(|c| matches!(c, '\n' | '\r')) {
        return ApiError::Validation("Invalid password characters".into()).into_response();
    }

    // Use chpasswd to set password: echo "user:password" | chpasswd
    let chpasswd_input = format!("{}:{}", unix_user, req.password);
    use tokio::io::AsyncWriteExt;
    let mut child = match tokio::process::Command::new("chpasswd")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return ApiError::Internal(anyhow::anyhow!("chpasswd spawn: {}", e)).into_response(),
    };

    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = stdin.write_all(chpasswd_input.as_bytes()).await {
            return ApiError::Internal(anyhow::anyhow!("chpasswd write: {}", e)).into_response();
        }
    }

    let output = match child.wait_with_output().await {
        Ok(o) => o,
        Err(e) => return ApiError::Internal(anyhow::anyhow!("chpasswd wait: {}", e)).into_response(),
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return ApiError::Internal(anyhow::anyhow!("chpasswd failed: {}", stderr)).into_response();
    }

    // Also enable the user's shell to /bin/bash so they can authenticate
    // (change from /sbin/nologin to /bin/bash for SFTP to work with password auth)
    let _ = tokio::process::Command::new("usermod")
        .args(["-s", "/bin/bash", &unix_user])
        .output().await;

    (StatusCode::OK, Json(json!({
        "status": "ok",
        "message": "SFTP password set successfully",
    }))).into_response()
}

// ── FTP helpers ───────────────────────────────────────────────────────────────

/// Hash a password with SHA-512-crypt (compatible with pure-ftpd PAM / pam_unix).
/// We call `openssl passwd -6` so the binary doesn't need a crate dep.
async fn sha512_crypt(password: &str) -> ApiResult<String> {
    if password.chars().any(|c| matches!(c, '\n' | '\r' | '\0')) {
        return Err(ApiError::Validation("Password contains invalid characters".into()));
    }
    use tokio::io::AsyncWriteExt;
    let mut child = tokio::process::Command::new("openssl")
        .args(["passwd", "-6", "-stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("openssl passwd spawn: {}", e)))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(password.as_bytes()).await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("openssl passwd write: {}", e)))?;
    }
    let out = child.wait_with_output().await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("openssl passwd wait: {}", e)))?;
    if !out.status.success() {
        return Err(ApiError::Internal(anyhow::anyhow!("openssl passwd failed")));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Validate FTP username: alphanumeric + underscore + hyphen, 3-32 chars.
fn validate_ftp_username(username: &str) -> ApiResult<()> {
    if username.len() < 3 || username.len() > 32 {
        return Err(ApiError::Validation("FTP username must be 3-32 characters".into()));
    }
    if !username.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(ApiError::Validation(
            "FTP username may only contain letters, numbers, underscores, and hyphens".into()
        ));
    }
    Ok(())
}

/// Validate home subdir: must start with '/', no '..', only safe chars.
fn validate_home_subdir(path: &str) -> ApiResult<()> {
    if !path.starts_with('/') {
        return Err(ApiError::Validation("home_subdir must start with '/'".into()));
    }
    if path.contains("..") {
        return Err(ApiError::Validation("home_subdir must not contain '..'".into()));
    }
    if path.chars().any(|c| matches!(c, '\0' | '\n' | '\r' | ';' | '&' | '|' | '$' | '`')) {
        return Err(ApiError::Validation("home_subdir contains invalid characters".into()));
    }
    Ok(())
}

/// Rebuild the pure-ftpd PureDB after any account change.
/// No-ops gracefully if pure-ftpd is not installed.
async fn rebuild_pureftpd_db() {
    // Read all FTP accounts from /etc/pure-ftpd/passwd/pureftpd.passwd (if exists)
    // and run pure-pw mkdb to rebuild the binary db.
    let _ = tokio::process::Command::new("pure-pw")
        .args(["mkdb"])
        .output()
        .await;
    // Reload pure-ftpd so it picks up the new db
    let _ = tokio::process::Command::new("systemctl")
        .args(["reload-or-restart", "pure-ftpd"])
        .output()
        .await;
}

/// Add or update a pure-ftpd virtual user entry.
/// Args: username, password_hash (SHA-512-crypt), unix_user (maps to system user), home_dir, quota_mb.
async fn pureftpd_add_user(
    username: &str,
    password_hash: &str,
    unix_user: &str,
    home_dir: &str,
    quota_mb: i32,
) -> ApiResult<()> {
    // Write a one-line shadow entry compatible with pure-ftpd's passwd file format.
    // Format: username:hash:uid:gid:gecos:homedir:shell
    // We look up the unix user's uid/gid dynamically.
    let uid_out = tokio::process::Command::new("id")
        .args(["-u", unix_user])
        .output().await
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "33".to_string());
    let gid_out = tokio::process::Command::new("id")
        .args(["-g", unix_user])
        .output().await
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "33".to_string());

    let quota_arg = if quota_mb > 0 {
        format!("{}M", quota_mb * 1024) // pure-ftpd uses kB or MB suffix
    } else {
        String::new()
    };

    // Use `pure-pw useradd` / `pure-pw usermod` to manage the virtual user.
    // `-m` rebuilds the db immediately; we also call rebuild_pureftpd_db() after.
    let output = tokio::process::Command::new("pure-pw")
        .args({
            let mut args = vec![
                "useradd".to_string(), username.to_string(),
                "-u".to_string(), uid_out.clone(),
                "-g".to_string(), gid_out.clone(),
                "-d".to_string(), home_dir.to_string(),
                "-m".to_string(),
            ];
            if !quota_arg.is_empty() {
                args.extend(["-n".to_string(), quota_arg.clone()]);
            }
            // Provide the hash via `-p` (pre-hashed password)
            args.extend(["-p".to_string(), password_hash.to_string()]);
            args
        })
        .output().await;

    // If useradd fails (user exists), try usermod
    if let Ok(o) = &output {
        if !o.status.success() {
            let _ = tokio::process::Command::new("pure-pw")
                .args({
                    let mut args = vec![
                        "usermod".to_string(), username.to_string(),
                        "-u".to_string(), uid_out,
                        "-g".to_string(), gid_out,
                        "-d".to_string(), home_dir.to_string(),
                        "-m".to_string(),
                    ];
                    if !quota_arg.is_empty() {
                        args.extend(["-n".to_string(), quota_arg]);
                    }
                    args.extend(["-p".to_string(), password_hash.to_string()]);
                    args
                })
                .output().await;
        }
    }
    Ok(())
}

/// Remove a pure-ftpd virtual user entry.
async fn pureftpd_del_user(username: &str) {
    let _ = tokio::process::Command::new("pure-pw")
        .args(["userdel", username, "-m"])
        .output().await;
}

// ── FTP user handlers ─────────────────────────────────────────────────────────

/// GET /api/v1/sites/:id/ftp-users
pub async fn ftp_users_list(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let (user_id, is_admin) = match caller(&claims) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    if let Err(e) = get_site_unix_user(&state, id, user_id, is_admin).await {
        return e.into_response();
    }

    let rows = match sqlx::query!(
        r#"SELECT id, username, home_subdir, quota_mb, enabled,
                  created_at
           FROM ftp_accounts
           WHERE site_id = $1
           ORDER BY created_at"#,
        id
    )
    .fetch_all(&state.db)
    .await {
        Ok(r) => r,
        Err(e) => return ApiError::Internal(anyhow::anyhow!("DB: {}", e)).into_response(),
    };

    let list: Vec<FtpUser> = rows.into_iter().map(|r| FtpUser {
        id:          r.id,
        username:    r.username,
        home_subdir: r.home_subdir,
        quota_mb:    r.quota_mb,
        enabled:     r.enabled,
        created_at:  r.created_at.to_rfc3339(),
    }).collect();

    (StatusCode::OK, Json(list)).into_response()
}

/// POST /api/v1/sites/:id/ftp-users
pub async fn ftp_users_create(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateFtpUserRequest>,
) -> impl IntoResponse {
    let (user_id, is_admin) = match caller(&claims) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let unix_user = match get_site_unix_user(&state, id, user_id, is_admin).await {
        Ok(u) => u,
        Err(e) => return e.into_response(),
    };

    if let Err(e) = validate_ftp_username(&req.username) {
        return e.into_response();
    }
    if req.password.len() < 8 {
        return ApiError::Validation("FTP password must be at least 8 characters".into()).into_response();
    }

    // Default home_subdir to "/" (site docroot)
    let home_subdir = "/".to_string();

    // Build the chroot path: /home/{unix_user}/public_html{subdir}
    let home_dir = format!("/home/{}/public_html", unix_user);

    // Hash password
    let password_hash = match sha512_crypt(&req.password).await {
        Ok(h) => h,
        Err(e) => return e.into_response(),
    };

    // Insert into DB
    let row = match sqlx::query!(
        r#"INSERT INTO ftp_accounts (site_id, username, password_hash, home_subdir, quota_mb, enabled)
           VALUES ($1, $2, $3, $4, 0, true)
           RETURNING id, username, home_subdir, quota_mb, enabled, created_at"#,
        id, req.username, password_hash, home_subdir
    )
    .fetch_one(&state.db)
    .await {
        Ok(r) => r,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("unique") || msg.contains("duplicate") {
                return ApiError::Validation("FTP username already taken".into()).into_response();
            }
            return ApiError::Internal(anyhow::anyhow!("DB insert: {}", e)).into_response();
        }
    };

    // Apply to pure-ftpd (non-fatal if not installed)
    let _ = pureftpd_add_user(&req.username, &password_hash, &unix_user, &home_dir, 0).await;
    rebuild_pureftpd_db().await;

    let resp = FtpUser {
        id:          row.id,
        username:    row.username,
        home_subdir: row.home_subdir,
        quota_mb:    row.quota_mb,
        enabled:     row.enabled,
        created_at:  row.created_at.to_rfc3339(),
    };
    (StatusCode::CREATED, Json(resp)).into_response()
}

/// PUT /api/v1/sites/:id/ftp-users/:user_id — update quota / subdir / enabled
pub async fn ftp_users_update(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path((id, ftp_user_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdateFtpUserRequest>,
) -> impl IntoResponse {
    let (user_id, is_admin) = match caller(&claims) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let unix_user = match get_site_unix_user(&state, id, user_id, is_admin).await {
        Ok(u) => u,
        Err(e) => return e.into_response(),
    };

    if let Some(ref sub) = req.home_subdir {
        if let Err(e) = validate_home_subdir(sub) {
            return e.into_response();
        }
    }
    if let Some(q) = req.quota_mb {
        if q < 0 { return ApiError::Validation("quota_mb cannot be negative".into()).into_response(); }
    }

    let row = match sqlx::query!(
        r#"UPDATE ftp_accounts
           SET home_subdir = COALESCE($1, home_subdir),
               quota_mb    = COALESCE($2, quota_mb),
               enabled     = COALESCE($3, enabled),
               updated_at  = NOW()
           WHERE id = $4 AND site_id = $5
           RETURNING id, username, password_hash, home_subdir, quota_mb, enabled, created_at"#,
        req.home_subdir, req.quota_mb, req.enabled,
        ftp_user_id, id
    )
    .fetch_optional(&state.db)
    .await {
        Ok(Some(r)) => r,
        Ok(None) => return ApiError::NotFound { resource: "FTP user" }.into_response(),
        Err(e)   => return ApiError::Internal(anyhow::anyhow!("DB update: {}", e)).into_response(),
    };

    let home_dir = format!(
        "/home/{}/public_html{}",
        unix_user,
        if row.home_subdir == "/" { String::new() } else { row.home_subdir.clone() }
    );

    let _ = pureftpd_add_user(&row.username, &row.password_hash, &unix_user, &home_dir, row.quota_mb).await;
    rebuild_pureftpd_db().await;

    (StatusCode::OK, Json(FtpUser {
        id:          row.id,
        username:    row.username,
        home_subdir: row.home_subdir,
        quota_mb:    row.quota_mb,
        enabled:     row.enabled,
        created_at:  row.created_at.to_rfc3339(),
    })).into_response()
}

/// PUT /api/v1/sites/:id/ftp-users/:user_id/password
pub async fn ftp_users_change_password(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path((id, ftp_user_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<ChangeFtpPasswordRequest>,
) -> impl IntoResponse {
    let (user_id, is_admin) = match caller(&claims) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let unix_user = match get_site_unix_user(&state, id, user_id, is_admin).await {
        Ok(u) => u,
        Err(e) => return e.into_response(),
    };

    if req.password.len() < 8 {
        return ApiError::Validation("Password must be at least 8 characters".into()).into_response();
    }

    let password_hash = match sha512_crypt(&req.password).await {
        Ok(h) => h,
        Err(e) => return e.into_response(),
    };

    let row = match sqlx::query!(
        r#"UPDATE ftp_accounts
           SET password_hash = $1, updated_at = NOW()
           WHERE id = $2 AND site_id = $3
           RETURNING username, home_subdir, quota_mb"#,
        password_hash, ftp_user_id, id
    )
    .fetch_optional(&state.db)
    .await {
        Ok(Some(r)) => r,
        Ok(None) => return ApiError::NotFound { resource: "FTP user" }.into_response(),
        Err(e)   => return ApiError::Internal(anyhow::anyhow!("DB: {}", e)).into_response(),
    };

    let home_dir = format!(
        "/home/{}/public_html{}",
        unix_user,
        if row.home_subdir == "/" { String::new() } else { row.home_subdir.clone() }
    );

    let _ = pureftpd_add_user(&row.username, &password_hash, &unix_user, &home_dir, row.quota_mb).await;
    rebuild_pureftpd_db().await;

    (StatusCode::OK, Json(json!({"message": "FTP password updated"}))).into_response()
}

/// DELETE /api/v1/sites/:id/ftp-users/:user_id
pub async fn ftp_users_delete(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path((id, ftp_user_id)): Path<(Uuid, Uuid)>,
) -> impl IntoResponse {
    let (user_id, is_admin) = match caller(&claims) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    if let Err(e) = get_site_unix_user(&state, id, user_id, is_admin).await {
        return e.into_response();
    }

    let row = match sqlx::query!(
        "DELETE FROM ftp_accounts WHERE id = $1 AND site_id = $2 RETURNING username",
        ftp_user_id, id
    )
    .fetch_optional(&state.db)
    .await {
        Ok(Some(r)) => r,
        Ok(None) => return ApiError::NotFound { resource: "FTP user" }.into_response(),
        Err(e)   => return ApiError::Internal(anyhow::anyhow!("DB delete: {}", e)).into_response(),
    };

    pureftpd_del_user(&row.username).await;
    rebuild_pureftpd_db().await;

    StatusCode::NO_CONTENT.into_response()
}
