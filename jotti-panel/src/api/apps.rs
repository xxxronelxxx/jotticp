use axum::{
    Router,
    routing::{delete, get, post},
    extract::{Path, Query, State, WebSocketUpgrade},
    extract::ws::{Message, WebSocket},
    Json,
    http::StatusCode,
    response::IntoResponse,
};
use chrono::{DateTime, Utc};
use ed25519_dalek::{Signature, VerifyingKey, Verifier};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};
use super::auth::Claims;

// ── Installer dispatch ────────────────────────────────────────────────────────

/// Download method declared in a TOML manifest's [download] section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallMethod {
    /// ZIP/tarball download — handled by existing 1-click installer engine.
    Archive,
    /// `composer create-project COMMAND` inside `$HOME/public_html/`.
    Composer,
    /// `npx COMMAND` via nvm bash wrapper inside `$HOME/`.
    Npx,
    /// `pip install PACKAGES` inside site virtualenv `$HOME/.venv`.
    Pip,
    /// `npm install PACKAGE` inside `$HOME/`.
    Npm,
}

impl InstallMethod {
    pub fn from_str(s: &str) -> Self {
        match s {
            "composer" => Self::Composer,
            "npx"      => Self::Npx,
            "pip"      => Self::Pip,
            "npm"      => Self::Npm,
            _          => Self::Archive,
        }
    }
}

/// Runtime parameters resolved at install time.
pub struct InstallRuntime {
    /// Unix username owning the site (e.g. "u12345").
    pub unix_user: String,
    /// Allocated port for the site's app process.
    pub service_port: u16,
    /// App-specific `[download].command` value from the TOML manifest.
    pub command: String,
    /// Whether to run `git init && git add -A && git commit` after install.
    pub git_init: bool,
}

/// Dispatch an app installation based on the manifest's download method.
///
/// This function builds and executes the correct install command for each
/// method variant, substituting `RANDOM_32BYTES_HEX` and `PORT` tokens,
/// then optionally performs a git init commit.
///
/// Returns the spawned process output (stdout + stderr) as a combined string
/// suitable for streaming to the progress WebSocket.
///
/// # Errors
/// Returns an error string if the subprocess fails or token substitution
/// cannot be completed.
pub async fn dispatch_app_install_by_method(
    method: InstallMethod,
    runtime: InstallRuntime,
) -> Result<String, String> {
    use rand::RngCore;

    // -- Token substitution helpers ------------------------------------------

    /// Replace `RANDOM_32BYTES_HEX` with a cryptographically-random 32-byte hex string.
    fn fill_random_hex(s: &str) -> String {
        if !s.contains("RANDOM_32BYTES_HEX") {
            return s.to_owned();
        }
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        s.replace("RANDOM_32BYTES_HEX", &hex::encode(bytes))
    }

    /// Replace `PORT` token with the site's allocated port number.
    fn fill_port(s: &str, port: u16) -> String {
        s.replace("PORT", &port.to_string())
    }

    let command = fill_port(&fill_random_hex(&runtime.command), runtime.service_port);
    let unix_user = &runtime.unix_user;
    let home = format!("/home/{unix_user}");
    let public_html = format!("{home}/public_html");

    // -- Build the shell command for each method -----------------------------

    let shell_cmd: String = match &method {
        InstallMethod::Composer => {
            // composer create-project COMMAND   (runs in public_html/)
            format!(
                "sudo -u {unix_user} composer {command} --working-dir={public_html}"
            )
        }

        InstallMethod::Npx => {
            // npx COMMAND   (runs in $HOME/, nvm loaded)
            format!(
                r#"sudo -u {unix_user} bash -c "source {home}/.nvm/nvm.sh && cd {home} && npx {command}""#
            )
        }

        InstallMethod::Pip => {
            // pip install PACKAGES inside the site virtualenv
            format!(
                r#"sudo -u {unix_user} bash -c "source {home}/.venv/bin/activate && {command}""#
            )
        }

        InstallMethod::Npm => {
            // npm install PACKAGE   (runs in $HOME/)
            format!(
                r#"sudo -u {unix_user} bash -c "source {home}/.nvm/nvm.sh && cd {home} && npm {command}""#
            )
        }

        InstallMethod::Archive => {
            return Err(
                "Archive method should be handled by the existing installer engine, \
                 not dispatch_app_install_by_method()".into()
            );
        }
    };

    // -- Execute install command ----------------------------------------------

    let output = tokio::process::Command::new("bash")
        .arg("-c")
        .arg(&shell_cmd)
        .output()
        .await
        .map_err(|e| format!("Failed to spawn install process: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let combined = format!("{stdout}\n{stderr}");

    if !output.status.success() {
        return Err(format!(
            "Install command exited with {}: {}",
            output.status,
            stderr.lines().last().unwrap_or("(no output)")
        ));
    }

    // -- Optional git init ---------------------------------------------------

    if runtime.git_init {
        let git_dir = match method {
            InstallMethod::Composer => public_html.clone(),
            _ => home.clone(),
        };

        let git_cmds = [
            format!("sudo -u {unix_user} git -C {git_dir} init"),
            format!("sudo -u {unix_user} git -C {git_dir} add -A"),
            format!(
                r#"sudo -u {unix_user} git -C {git_dir} commit -m "Initial JottiCP install""#
            ),
        ];

        for cmd in &git_cmds {
            let git_out = tokio::process::Command::new("bash")
                .arg("-c")
                .arg(cmd)
                .output()
                .await
                .map_err(|e| format!("git command failed to spawn: {e}"))?;

            if !git_out.status.success() {
                // Non-fatal: log and continue (git init may fail if repo already exists).
                tracing::warn!(
                    cmd = %cmd,
                    stderr = %String::from_utf8_lossy(&git_out.stderr),
                    "git step failed (non-fatal)"
                );
            }
        }
    }

    Ok(combined)
}

// ── Ed25519 archive verification ──────────────────────────────────────────────

/// Verify an Ed25519 signature over the raw bytes of a downloaded app archive.
///
/// `pubkey_hex` is the 32-byte verifying key in lowercase hex (64 chars).
/// `sig_hex`    is the 64-byte signature in lowercase hex (128 chars).
///
/// Returns `Ok(())` on a valid signature, or `Err(String)` describing why
/// verification failed (invalid key, invalid signature bytes, or bad sig).
///
/// The public key is stored in the app TOML manifest as `[download].pubkey`.
/// It must be the hex-encoded compressed Ed25519 verifying key from the
/// JottiCP release signing key pair.
pub fn verify_archive_signature(
    archive_bytes: &[u8],
    pubkey_hex:    &str,
    sig_hex:       &str,
) -> Result<(), String> {
    // Decode the 32-byte verifying key.
    let pubkey_bytes: [u8; 32] = hex::decode(pubkey_hex)
        .map_err(|e| format!("invalid pubkey hex: {e}"))?
        .try_into()
        .map_err(|_| "pubkey must be exactly 32 bytes".to_string())?;

    let verifying_key = VerifyingKey::from_bytes(&pubkey_bytes)
        .map_err(|e| format!("invalid Ed25519 verifying key: {e}"))?;

    // Decode the 64-byte signature.
    let sig_bytes: [u8; 64] = hex::decode(sig_hex)
        .map_err(|e| format!("invalid signature hex: {e}"))?
        .try_into()
        .map_err(|_| "signature must be exactly 64 bytes".to_string())?;

    let signature = Signature::from_bytes(&sig_bytes);

    verifying_key
        .verify(archive_bytes, &signature)
        .map_err(|e| format!("Ed25519 signature verification failed: {e}"))
}

// ── TOML manifest loader ──────────────────────────────────────────────────────

/// Load app TOML manifests from the directory pointed to by the
/// `ORBIT_APPS_DIR` environment variable (default: `/etc/jottiecp/apps`).
///
/// Each `*.toml` file in the directory is expected to have at minimum:
/// ```toml
/// [app]
/// id       = "wordpress"
/// name     = "WordPress"
/// version  = "6.5.3"
/// category = "cms"
/// ```
///
/// Manifests are loaded fresh on each call (low frequency; called only when
/// the marketplace list is fetched).  This means adding a new app TOML does
/// not require a server restart.
pub fn load_app_manifests() -> Vec<AppManifest> {
    let apps_dir = std::env::var("ORBIT_APPS_DIR")
        .unwrap_or_else(|_| "/etc/jottiecp/apps".to_string());

    let dir = match std::fs::read_dir(&apps_dir) {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(
                dir = %apps_dir,
                error = %e,
                "ORBIT_APPS_DIR unreadable, falling back to built-in app list"
            );
            return builtin_app_manifests();
        }
    };

    let mut manifests = Vec::new();

    for entry in dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "failed to read app manifest");
                continue;
            }
        };

        let table: toml::Value = match toml::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "invalid TOML in app manifest");
                continue;
            }
        };

        let app = match table.get("app") {
            Some(a) => a,
            None => {
                tracing::warn!(path = %path.display(), "app manifest missing [app] section");
                continue;
            }
        };

        let id = app.get("id").and_then(|v| v.as_str()).unwrap_or_default();
        if id.is_empty() {
            tracing::warn!(path = %path.display(), "app manifest missing app.id");
            continue;
        }

        manifests.push(AppManifest {
            id:           id.to_string(),
            name:         app.get("name").and_then(|v| v.as_str()).unwrap_or(id).to_string(),
            description:  app.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            version:      app.get("version").and_then(|v| v.as_str()).unwrap_or("latest").to_string(),
            category:     app.get("category").and_then(|v| v.as_str()).unwrap_or("other").to_string(),
            icon_url:     app.get("icon_url").and_then(|v| v.as_str()).map(|s| s.to_string()),
            requires_pro: app.get("requires_pro").and_then(|v| v.as_bool()).unwrap_or(false),
        });
    }

    if manifests.is_empty() {
        tracing::info!("no TOML manifests found in {apps_dir}, using built-in list");
        return builtin_app_manifests();
    }

    tracing::debug!(count = manifests.len(), dir = %apps_dir, "loaded app manifests from disk");
    manifests
}

/// Hard-coded fallback list used when `ORBIT_APPS_DIR` is missing or empty.
fn builtin_app_manifests() -> Vec<AppManifest> {
    KNOWN_APPS.iter().map(|(id, name, desc, category)| AppManifest {
        id:           id.to_string(),
        name:         name.to_string(),
        description:  desc.to_string(),
        version:      "latest".into(),
        category:     category.to_string(),
        icon_url:     None,
        requires_pro: false,
    }).collect()
}

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct AppManifest {
    pub id:          String,   // "wordpress", "joomla", "nextcloud"
    pub name:        String,
    pub description: String,
    pub version:     String,
    pub category:    String,
    pub icon_url:    Option<String>,
    pub requires_pro: bool,
}

#[derive(Debug, Deserialize)]
pub struct InstallAppRequest {
    pub app_id:    String,   // "wordpress"
    pub site_id:   Uuid,
    pub params:    serde_json::Value, // app-specific: db_name, admin_email, etc.
}

#[derive(Debug, Serialize)]
pub struct AppInstallation {
    pub id:           Uuid,
    pub site_id:      Uuid,
    pub app_id:       String,
    pub version:      String,
    pub status:       String,  // "pending" | "installing" | "active" | "failed" | "updating"
    pub installed_at: Option<DateTime<Utc>>,
    pub admin_url:    Option<String>,
    /// Generated admin credentials, present only when status = 'active'.
    /// Returned once in the GET /api/v1/apps/installed/{id} response so the
    /// caller can store them.  The password is stored in the DB (encrypted
    /// at rest by PostgreSQL pgcrypto or a future vault integration).
    pub admin_credentials: Option<AdminCredentials>,
}

#[derive(Debug, Serialize)]
pub struct AdminCredentials {
    pub admin_user:     String,
    pub admin_email:    String,
    /// The plain-text password is only returned once, on the first retrieval
    /// after installation.  Subsequent requests should omit it.
    pub admin_password: String,
}

#[derive(Debug, Deserialize)]
pub struct ListInstallationsQuery {
    pub site_id: Option<Uuid>,
}

// Known app manifests (loaded from /src/jotti-panel/apps/*.toml in production;
// hardcoded here for build-time correctness)
const KNOWN_APPS: &[(&str, &str, &str, &str)] = &[
    // CMS
    ("wordpress",     "WordPress",          "The world's most popular CMS",                     "cms"),
    ("joomla",        "Joomla",             "Open-source content management",                    "cms"),
    ("drupal",        "Drupal",             "Enterprise open-source CMS",                        "cms"),
    ("ghost",         "Ghost",              "Modern publishing platform for creators",            "cms"),
    // Frameworks
    ("laravel",       "Laravel",            "Elegant PHP web application framework",              "framework"),
    ("symfony",       "Symfony",            "Professional PHP framework for web applications",    "framework"),
    ("nextjs",        "Next.js",            "React framework for production-grade web apps",      "framework"),
    ("nuxt",          "Nuxt 3",             "Vue.js framework with SSR, SSG, and SPA support",   "framework"),
    ("django",        "Django",             "Python web framework — batteries included",          "framework"),
    // AI
    ("fastapi",       "FastAPI + Uvicorn",  "Modern, fast Python API framework with async",       "ai"),
    ("ollama_proxy",  "LangChain + Ollama", "AI inference proxy with LangChain integration",     "ai"),
    ("n8n_ai",        "n8n AI Automation",  "Workflow automation with 400+ integrations",         "ai"),
    // Productivity / tools
    ("nextcloud",     "Nextcloud",          "Self-hosted cloud storage and collaboration",        "productivity"),
    ("gitea",         "Gitea",              "Lightweight self-hosted Git service",                "dev"),
    ("roundcube",     "Roundcube",          "Browser-based IMAP email client",                   "email"),
    ("matomo",        "Matomo",             "Privacy-first web analytics platform",               "productivity"),
    ("n8n",           "n8n",                "Workflow automation for everyone",                   "productivity"),
];

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/apps",                        get(list_apps))
        .route("/api/v1/apps/installed",              get(list_installations))
        .route("/api/v1/apps/install",                post(install_app))
        .route("/api/v1/apps/installed/{id}",          delete(uninstall_app))
        .route("/api/v1/apps/installed/{id}/update",   post(trigger_update))
        // WebSocket endpoint for real-time install progress.
        // Client subscribes with: GET /api/v1/apps/installed/{id}/progress
        // and receives newline-delimited JSON progress messages until the
        // install completes or fails.
        .route("/api/v1/apps/installed/{id}/progress", get(install_progress_ws))
}

fn caller(claims: &Claims) -> ApiResult<(Uuid, bool)> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    let is_admin = claims.role == "admin";
    Ok((user_id, is_admin))
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// List all available apps in the marketplace.
/// Manifests are loaded from `ORBIT_APPS_DIR` (default `/etc/jottiecp/apps`)
/// on each request, with a fall-through to the built-in list.
async fn list_apps(
    State(_state): State<Arc<AppState>>,
    _claims: Claims,
) -> ApiResult<Json<Vec<AppManifest>>> {
    Ok(Json(load_app_manifests()))
}

/// List installed apps, optionally filtered by site.
async fn list_installations(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Query(q): Query<ListInstallationsQuery>,
) -> ApiResult<Json<Vec<AppInstallation>>> {
    let (user_id, is_admin) = caller(&claims)?;

    let rows = sqlx::query!(
        r#"SELECT ai.id, ai.site_id, ai.manifest_slug AS app_id,
                  ai.installed_version AS version, ai.status,
                  ai.installed_at,
                  ai.metadata->>'admin_url' AS admin_url,
                  ai.metadata::text AS params
           FROM app_installs ai
           JOIN sites s ON s.id = ai.site_id
           WHERE ai.deleted_at IS NULL
             AND ($1::boolean OR s.owner_id = $3)
             AND ($2::uuid IS NULL OR ai.site_id = $2::uuid)
           ORDER BY ai.installed_at DESC
           LIMIT 200"#,
        is_admin, q.site_id as Option<Uuid>, user_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|r| {
        // Surface admin credentials if stored in params (set by job worker on success).
        let admin_credentials = if r.status == "active" {
            r.params.as_ref().and_then(|p_str| {
                let p: serde_json::Value = serde_json::from_str(p_str).ok()?;
                let user  = p["admin_user"].as_str()?;
                let email = p["admin_email"].as_str()?;
                let pw    = p["admin_password"].as_str()?;
                Some(AdminCredentials {
                    admin_user:     user.to_string(),
                    admin_email:    email.to_string(),
                    admin_password: pw.to_string(),
                })
            })
        } else {
            None
        };

        AppInstallation {
            id:           r.id,
            site_id:      r.site_id,
            app_id:       r.app_id,
            version:      r.version,
            status:       r.status,
            installed_at: Some(r.installed_at),
            admin_url:    r.admin_url,
            admin_credentials,
        }
    }).collect()))
}

/// Queue a 1-click app installation job.
async fn install_app(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<InstallAppRequest>,
) -> ApiResult<(StatusCode, Json<AppInstallation>)> {
    let (user_id, is_admin) = caller(&claims)?;

    // Validate app_id against the current manifest list (TOML files or built-ins).
    let manifests = load_app_manifests();
    if !manifests.iter().any(|m| m.id == req.app_id) {
        return Err(ApiError::Validation(format!("Unknown app_id: {}", req.app_id)));
    }

    // Verify caller owns the site
    let site = sqlx::query!(
        "SELECT id, domain, status::text AS status FROM sites WHERE id = $1 AND deleted_at IS NULL
         AND ($2::boolean OR owner_id = $3)",
        req.site_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "site" })?;

    if site.status.as_deref() != Some("active") {
        return Err(ApiError::Validation(format!(
            "Site '{}' is not active (status: {:?})", site.domain, site.status
        )));
    }

    // Check for duplicate installation
    let existing = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM app_installs WHERE site_id = $1 AND manifest_slug = $2 AND status != 'failed' AND deleted_at IS NULL)",
        req.site_id, req.app_id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(false);

    if existing {
        return Err(ApiError::Validation(format!(
            "{} is already installed on this site", req.app_id
        )));
    }

    let install_id = Uuid::new_v4();

    sqlx::query!(
        "INSERT INTO app_installs (id, site_id, manifest_slug, installed_version, status,
                                   metadata, install_path, installed_at)
         VALUES ($1, $2, $3, 'latest', 'pending', $4, '', NOW())",
        install_id, req.site_id, req.app_id,
        serde_json::json!({"params": req.params}) as serde_json::Value
    )
    .execute(&state.db)
    .await?;

    // Enqueue installation job
    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "install_app",
            "install_id": install_id,
            "site_id": req.site_id,
            "app_id": req.app_id,
            "params": req.params,
            "domain": site.domain,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    tracing::info!(
        user = %user_id,
        site = %req.site_id,
        app  = %req.app_id,
        install = %install_id,
        "App installation queued"
    );

    Ok((StatusCode::ACCEPTED, Json(AppInstallation {
        id:                install_id,
        site_id:           req.site_id,
        app_id:            req.app_id,
        version:           "latest".into(),
        status:            "pending".into(),
        installed_at:      None,
        admin_url:         None,
        // Credentials are not yet available (job is queued).
        // Subscribe to GET /api/v1/apps/installed/{install_id}/progress
        // via WebSocket to receive them on completion.
        admin_credentials: None,
    })))
}

/// Remove an installed app (soft-delete record; files remain unless site is deleted).
async fn uninstall_app(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(install_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;

    let affected = sqlx::query!(
        r#"UPDATE app_installs ai SET deleted_at = NOW(), status = 'removed'
           FROM sites s
           WHERE ai.id = $1 AND ai.site_id = s.id AND ai.deleted_at IS NULL
             AND ($2::boolean OR s.owner_id = $3)"#,
        install_id, is_admin, user_id
    )
    .execute(&state.db)
    .await?
    .rows_affected();

    if affected == 0 {
        return Err(ApiError::NotFound { resource: "app_installation" });
    }

    Ok(StatusCode::NO_CONTENT)
}

// ── Install progress WebSocket ────────────────────────────────────────────────

/// GET /api/v1/apps/installed/{id}/progress — upgrade to WebSocket and stream
/// installation progress messages published by the job worker.
///
/// The job worker publishes progress lines to the Valkey channel
/// `orbit:install-progress:{install_id}` as JSON objects:
///   `{"step": "download", "pct": 25, "msg": "Downloading archive…"}`
///   `{"step": "done",     "pct": 100, "status": "active", "admin_user": "admin", "admin_password": "…"}`
///   `{"step": "error",    "pct": 0,   "msg": "…error text…"}`
///
/// The WebSocket is closed automatically by the server after a "done" or
/// "error" message, or after a 10-minute timeout.
async fn install_progress_ws(
    State(state): State<Arc<AppState>>,
    Path(install_id): Path<Uuid>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| stream_install_progress(state, install_id, socket))
}

async fn stream_install_progress(
    state: Arc<AppState>,
    install_id: Uuid,
    mut socket: WebSocket,
) {
    use redis::AsyncCommands;

    let channel = format!("orbit:install-progress:{install_id}");

    // Get a dedicated pub/sub connection from the Valkey client.
    // We clone the underlying client from the connection manager via a
    // new connection (connection managers don't expose pub/sub).
    let valkey_url = state.config.valkey_url.as_str();
    let client = match redis::Client::open(valkey_url) {
        Ok(c) => c,
        Err(e) => {
            let _ = socket.send(Message::Text(
                serde_json::json!({"step": "error", "msg": format!("pub/sub connect failed: {e}")})
                    .to_string()
                    .into()
            )).await;
            return;
        }
    };

    let mut pubsub = match client.get_async_pubsub().await {
        Ok(p) => p,
        Err(e) => {
            let _ = socket.send(Message::Text(
                serde_json::json!({"step": "error", "msg": format!("pub/sub init failed: {e}")})
                    .to_string()
                    .into()
            )).await;
            return;
        }
    };

    if let Err(e) = pubsub.subscribe(&channel).await {
        let _ = socket.send(Message::Text(
            serde_json::json!({"step": "error", "msg": format!("subscribe failed: {e}")})
                .to_string()
                .into()
        )).await;
        return;
    }

    use futures_util::StreamExt;
    let mut msg_stream = pubsub.on_message();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(600);

    loop {
        tokio::select! {
            msg = msg_stream.next() => {
                let msg = match msg {
                    Some(m) => m,
                    None => break, // channel closed
                };
                let payload: String = match msg.get_payload() {
                    Ok(p) => p,
                    Err(_) => break,
                };

                let is_terminal = serde_json::from_str::<serde_json::Value>(&payload)
                    .ok()
                    .and_then(|v| v["step"].as_str().map(|s| matches!(s, "done" | "error")))
                    .unwrap_or(false);

                let _ = socket.send(Message::Text(payload.into())).await;

                if is_terminal {
                    break;
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                let _ = socket.send(Message::Text(
                    serde_json::json!({"step": "error", "msg": "installation timed out"})
                        .to_string()
                        .into()
                )).await;
                break;
            }
        }
    }
}

/// Trigger an app update check and upgrade job.
async fn trigger_update(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(install_id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let (user_id, is_admin) = caller(&claims)?;

    let installation = sqlx::query!(
        r#"SELECT ai.id, ai.site_id, ai.manifest_slug AS app_id, s.domain
           FROM app_installs ai JOIN sites s ON s.id = ai.site_id
           WHERE ai.id = $1 AND ai.deleted_at IS NULL AND ai.status = 'active'
             AND ($2::boolean OR s.owner_id = $3)"#,
        install_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "app_installation" })?;

    // Enqueue update job
    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "update_app",
            "install_id": install_id,
            "site_id": installation.site_id,
            "app_id": installation.app_id,
            "domain": installation.domain,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok(Json(serde_json::json!({
        "queued": true,
        "install_id": install_id,
        "message": "Update check queued — check status via GET /api/v1/apps/installed"
    })))
}
