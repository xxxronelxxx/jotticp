use axum::{
    Router,
    routing::{delete, get, post},
    extract::{Path, State},
    Json,
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};
use super::auth::Claims;

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct SshKeyResponse {
    pub id:          Uuid,
    pub label:       Option<String>,
    pub fingerprint: String,
    pub key_type:    String,
    pub created_at:  DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct AddSshKeyRequest {
    pub label:      Option<String>,
    pub public_key: String,
}

#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub message: String,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/ssh-keys",                         get(list_keys).post(add_key))
        .route("/api/v1/ssh-keys/{id}",                     delete(delete_key))
        .route("/api/v1/ssh-keys/{id}/disable-password",    post(disable_password_auth))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Validate SSH public key format.
/// Must start with a known key type prefix.
fn validate_ssh_key(key: &str) -> ApiResult<String> {
    let key = key.trim();
    let valid_prefixes = [
        "ssh-rsa",
        "ssh-ed25519",
        "ecdsa-sha2-nistp256",
        "ecdsa-sha2-nistp384",
        "ecdsa-sha2-nistp521",
        "sk-ssh-ed25519@openssh.com",
        "sk-ecdsa-sha2-nistp256@openssh.com",
    ];

    let key_type = valid_prefixes
        .iter()
        .find(|&&prefix| key.starts_with(prefix))
        .copied()
        .ok_or_else(|| ApiError::Validation(
            "Public key must start with ssh-rsa, ssh-ed25519, or ecdsa-sha2-*".into()
        ))?;

    // Basic structural check: must have at least 2 space-separated parts (type + base64)
    let parts: Vec<&str> = key.splitn(3, ' ').collect();
    if parts.len() < 2 || parts[1].is_empty() {
        return Err(ApiError::Validation("Invalid SSH public key format".into()));
    }

    // The base64 portion must only contain valid base64 characters
    let b64_re = Regex::new(r"^[A-Za-z0-9+/]+=*$").unwrap();
    if !b64_re.is_match(parts[1]) {
        return Err(ApiError::Validation("Invalid base64 in SSH public key".into()));
    }

    // Reject keys with embedded newlines or shell metacharacters
    if key.chars().any(|c| matches!(c, '\n' | '\r' | ';' | '&' | '|' | '$' | '`')) {
        return Err(ApiError::Validation("SSH key contains invalid characters".into()));
    }

    Ok(key_type.to_string())
}

/// Compute the SSH key fingerprint using `ssh-keygen -lf -`.
async fn compute_fingerprint(public_key: &str) -> ApiResult<String> {
    use tokio::process::Command;

    // Write key to a temp file then fingerprint it
    let tmp = format!("/tmp/jotticp-key-{}.pub", Uuid::new_v4());
    tokio::fs::write(&tmp, public_key).await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Temp key write: {}", e)))?;

    let output = Command::new("ssh-keygen")
        .args(["-l", "-f", &tmp])
        .output()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("ssh-keygen spawn: {}", e)))?;

    // Always remove temp file, ignore errors
    let _ = tokio::fs::remove_file(&tmp).await;

    if !output.status.success() {
        return Err(ApiError::Validation("Could not parse SSH public key — is it a valid key?".into()));
    }

    // Output format: "2048 SHA256:FINGERPRINT comment (RSA)"
    let stdout = String::from_utf8_lossy(&output.stdout);
    let fingerprint = stdout
        .split_whitespace()
        .nth(1)
        .map(|s| s.to_string())
        .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("Unexpected ssh-keygen output")))?;

    Ok(fingerprint)
}

/// Get the home directory for a user by running `getent passwd USERNAME`.
async fn get_user_home(username: &str) -> anyhow::Result<String> {
    let output = tokio::process::Command::new("getent")
        .args(["passwd", username])
        .output()
        .await?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Format: username:x:uid:gid:gecos:home:shell
    let home = stdout
        .split(':')
        .nth(5)
        .map(|s| s.trim().to_string())
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory for {}", username))?;
    Ok(home)
}

/// Append a public key to authorized_keys atomically (append-mode open).
async fn append_authorized_key(home: &str, public_key: &str) -> ApiResult<()> {
    let ssh_dir = format!("{}/.ssh", home);
    let auth_keys = format!("{}/authorized_keys", ssh_dir);

    tokio::fs::create_dir_all(&ssh_dir)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Create .ssh dir: {}", e)))?;

    // Set .ssh directory permissions to 700
    tokio::process::Command::new("chmod")
        .args(["700", &ssh_dir])
        .output()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("chmod .ssh: {}", e)))?;

    // Append the key followed by newline
    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&auth_keys)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Open authorized_keys: {}", e)))?;

    let mut line = public_key.trim().to_string();
    line.push('\n');
    file.write_all(line.as_bytes())
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Write authorized_keys: {}", e)))?;

    // Set authorized_keys permissions to 600
    tokio::process::Command::new("chmod")
        .args(["600", &auth_keys])
        .output()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("chmod authorized_keys: {}", e)))?;

    Ok(())
}

/// Remove a specific public key from authorized_keys (match by exact line).
/// Uses an atomic write (temp file + rename) to prevent corruption.
async fn remove_from_authorized_keys(home: &str, public_key: &str) -> ApiResult<()> {
    let auth_keys = format!("{}/.ssh/authorized_keys", home);
    let content = tokio::fs::read_to_string(&auth_keys).await.unwrap_or_default();

    let normalized_key = public_key.trim();
    let new_lines: Vec<&str> = content
        .lines()
        .filter(|line| line.trim() != normalized_key)
        .collect();

    let mut out = new_lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }

    // Atomic write: temp file alongside the target, then rename.
    // A crash during write cannot leave authorized_keys in a partial state.
    let tmp_path = format!("{}.tmp.{}", auth_keys, std::process::id());
    tokio::fs::write(&tmp_path, &out)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Write authorized_keys (tmp): {}", e)))?;

    // Preserve 0600 permissions on the temp file before rename
    tokio::process::Command::new("chmod")
        .args(["600", &tmp_path])
        .output()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("chmod tmp authorized_keys: {}", e)))?;

    tokio::fs::rename(&tmp_path, &auth_keys)
        .await
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp_path);
            ApiError::Internal(anyhow::anyhow!("Rename authorized_keys: {}", e))
        })?;

    Ok(())
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /api/v1/ssh-keys — list caller's SSH keys
async fn list_keys(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<Vec<SshKeyResponse>>> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;

    let rows = sqlx::query!(
        r#"
        SELECT id, label, fingerprint, public_key, created_at
        FROM ssh_keys
        WHERE user_id = $1
        ORDER BY created_at DESC
        "#,
        user_id
    )
    .fetch_all(&state.db)
    .await?;

    let keys = rows.into_iter().map(|row| {
        let key_type = row.public_key
            .split_whitespace()
            .next()
            .unwrap_or("unknown")
            .to_string();
        SshKeyResponse {
            id:          row.id,
            label:       row.label,
            fingerprint: row.fingerprint,
            key_type,
            created_at:  row.created_at,
        }
    }).collect();

    Ok(Json(keys))
}

/// POST /api/v1/ssh-keys — add a new SSH key
async fn add_key(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<AddSshKeyRequest>,
) -> ApiResult<(StatusCode, Json<SshKeyResponse>)> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;

    let key_type = validate_ssh_key(&req.public_key)?;
    let public_key = req.public_key.trim().to_string();

    // Compute fingerprint first — this also validates the key is parseable
    let fingerprint = compute_fingerprint(&public_key).await?;

    // Check for duplicate fingerprint for this user
    let exists: bool = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM ssh_keys WHERE user_id = $1 AND fingerprint = $2)",
        user_id, fingerprint
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(false);

    if exists {
        return Err(ApiError::Validation("This SSH key is already registered".into()));
    }

    // Validate label if provided
    if let Some(ref label) = req.label {
        if label.len() > 100 {
            return Err(ApiError::Validation("Label must be 100 characters or fewer".into()));
        }
    }

    // Get the admin's Unix username from DB
    let user = sqlx::query!(
        r#"SELECT email, role::text AS "role!" FROM users WHERE id = $1"#,
        user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::Unauthorized)?;

    // For admin users, write to /home/ADMIN/.ssh/authorized_keys
    // The username is derived from the email local part (same as provisioning convention)
    // For safety, look up via admin's configured unix_user or fall back to "admin"
    let unix_user = if user.role == "admin" {
        // Admins use their panel admin account; default to /root for root admin
        "root".to_string()
    } else {
        return Err(ApiError::Forbidden("SSH key management is only available for admin accounts".into()));
    };

    let home = if unix_user == "root" {
        "/root".to_string()
    } else {
        get_user_home(&unix_user)
            .await
            .map_err(|e| ApiError::Internal(e))?
    };

    append_authorized_key(&home, &public_key).await?;

    // Persist to DB
    let id = sqlx::query_scalar!(
        r#"
        INSERT INTO ssh_keys (user_id, label, public_key, fingerprint)
        VALUES ($1, $2, $3, $4)
        RETURNING id
        "#,
        user_id, req.label, public_key, fingerprint
    )
    .fetch_one(&state.db)
    .await?;

    tracing::info!(admin = %claims.sub, fingerprint = %fingerprint, "SSH key added");

    Ok((StatusCode::CREATED, Json(SshKeyResponse {
        id,
        label:       req.label,
        fingerprint,
        key_type,
        created_at:  Utc::now(),
    })))
}

/// DELETE /api/v1/ssh-keys/{id} — remove an SSH key
async fn delete_key(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(key_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;

    // Fetch key — only owner can delete
    let key = sqlx::query!(
        "SELECT public_key FROM ssh_keys WHERE id = $1 AND user_id = $2",
        key_id, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "SSH key" })?;

    // Determine home directory (same logic as add_key)
    let user = sqlx::query!(r#"SELECT role::text AS "role!" FROM users WHERE id = $1"#, user_id)
        .fetch_one(&state.db)
        .await?;

    let home = if user.role == "admin" {
        "/root".to_string()
    } else {
        return Err(ApiError::Forbidden("SSH key management is only available for admin accounts".into()));
    };

    remove_from_authorized_keys(&home, &key.public_key).await?;

    sqlx::query!("DELETE FROM ssh_keys WHERE id = $1", key_id)
        .execute(&state.db)
        .await?;

    tracing::info!(admin = %claims.sub, key_id = %key_id, "SSH key removed");
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/ssh-keys/{id}/disable-password
/// Sets `PasswordAuthentication no` in /etc/ssh/sshd_config.d/jotticp.conf and reloads sshd.
/// Requires the caller to have at least one active SSH key (to avoid lockout).
async fn disable_password_auth(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(_key_id): Path<Uuid>,
) -> ApiResult<Json<MessageResponse>> {
    if claims.role != "admin" {
        return Err(ApiError::Forbidden("Admin access required".into()));
    }

    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;

    // Verify caller has at least one SSH key — prevent lockout
    let key_count: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM ssh_keys WHERE user_id = $1",
        user_id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(0);

    if key_count == 0 {
        return Err(ApiError::Validation(
            "Cannot disable password auth: no SSH keys are registered. Add a key first.".into()
        ));
    }

    let sshd_conf = "/etc/ssh/sshd_config.d/jotticp.conf";
    let conf_content = "# Managed by JottiCP — do not edit manually\nPasswordAuthentication no\nChallengeResponseAuthentication no\n";

    // Ensure directory exists
    tokio::fs::create_dir_all("/etc/ssh/sshd_config.d")
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Create sshd_config.d: {}", e)))?;

    tokio::fs::write(sshd_conf, conf_content)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Write sshd config: {}", e)))?;

    // Validate sshd config before reloading
    let test = tokio::process::Command::new("sshd")
        .args(["-t"])
        .output()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("sshd -t spawn: {}", e)))?;

    if !test.status.success() {
        // Revert
        let _ = tokio::fs::remove_file(sshd_conf).await;
        let stderr = String::from_utf8_lossy(&test.stderr);
        return Err(ApiError::Validation(format!("sshd config invalid: {}", stderr)));
    }

    // Reload sshd
    tokio::process::Command::new("sudo")
        .args(["systemctl", "reload", "sshd"])
        .output()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("sshd reload: {}", e)))?;

    tracing::info!(admin = %claims.sub, "Password authentication disabled");
    Ok(Json(MessageResponse {
        message: "Password authentication disabled. SSH key login is now required.".into(),
    }))
}
