use axum::{
    Router,
    routing::{get, post, delete},
    extract::{Path, State},
    Json,
    http::StatusCode,
};
use axum_extra::extract::Multipart;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};
use super::auth::Claims;

// ── Constants ─────────────────────────────────────────────────────────────────

const MAX_PLUGIN_BYTES: usize = 50 * 1024 * 1024; // 50 MB
const PLUGIN_INSTALL_DIR: &str = "/var/jotticp/plugins";

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct PluginResponse {
    pub slug:         String,
    pub name:         String,
    pub version:      String,
    pub author:       String,
    pub description:  String,
    pub tags:         Vec<String>,
    pub homepage:     Option<String>,
    pub icon_url:     Option<String>,
    pub installed:    bool,
    pub enabled:      bool,
    pub manifest:     serde_json::Value,
    pub requires_plan: String,
    pub installed_at: DateTime<Utc>,
}

/// Minimal TOML manifest fields we care about.
#[derive(Debug, Deserialize)]
struct PluginManifest {
    plugin: PluginMeta,
    #[serde(default)]
    events: std::collections::HashMap<String, String>,
    #[serde(default)]
    permissions: PluginPermissions,
}

#[derive(Debug, Deserialize)]
struct PluginMeta {
    slug:         String,
    name:         String,
    version:      String,
    #[serde(default)]
    author:       String,
    #[serde(default)]
    description:  String,
    #[serde(default)]
    tags:         Vec<String>,
    homepage:     Option<String>,
    #[serde(default)]
    requires_plan: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct PluginPermissions {
    #[serde(default)]
    network: bool,
    #[serde(default)]
    exec:    bool,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/plugins",               get(list_plugins))
        .route("/api/v1/plugins/upload",         post(upload_plugin))
        .route("/api/v1/plugins/{slug}/install", post(install_plugin))
        .route("/api/v1/plugins/{slug}/enable",  post(enable_plugin))
        .route("/api/v1/plugins/{slug}/disable", post(disable_plugin))
        .route("/api/v1/plugins/{slug}",         delete(uninstall_plugin))
}

// ── Access helpers ────────────────────────────────────────────────────────────

fn require_admin(claims: &Claims) -> ApiResult<()> {
    if claims.role != "admin" {
        return Err(ApiError::Forbidden("Admin access required".into()));
    }
    Ok(())
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn list_plugins(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<Vec<PluginResponse>>> {
    require_admin(&claims)?;

    let rows = sqlx::query!(
        r#"SELECT slug, name, version, author, description, tags,
                  homepage, icon_url, enabled, manifest, requires_plan, installed_at
           FROM plugins
           ORDER BY installed_at DESC"#
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|r| PluginResponse {
        slug:         r.slug,
        name:         r.name,
        version:      r.version,
        author:       r.author,
        description:  r.description,
        tags:         r.tags,
        homepage:     r.homepage,
        icon_url:     r.icon_url,
        installed:    true,
        enabled:      r.enabled,
        manifest:     r.manifest,
        requires_plan: r.requires_plan,
        installed_at: r.installed_at,
    }).collect()))
}

/// Upload a .tar.gz plugin archive containing `jotti-plugin.toml`.
async fn upload_plugin(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    mut multipart: Multipart,
) -> ApiResult<(StatusCode, Json<PluginResponse>)> {
    require_admin(&claims)?;

    let mut plugin_bytes: Option<Vec<u8>> = None;
    let mut filename = String::from("plugin.tar.gz");

    while let Some(field) = multipart.next_field().await
        .map_err(|e| ApiError::Validation(format!("Multipart error: {}", e)))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "plugin" || name == "file" {
            filename = field.file_name()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "plugin.tar.gz".into());
            let data = field.bytes().await
                .map_err(|e| ApiError::Validation(format!("Read error: {}", e)))?;
            if data.len() > MAX_PLUGIN_BYTES {
                return Err(ApiError::Validation("Plugin archive must be ≤ 50 MB".into()));
            }
            // Validate gzip magic bytes
            if data.len() < 2 || data[0] != 0x1f || data[1] != 0x8b {
                return Err(ApiError::Validation("Plugin file must be a .tar.gz archive".into()));
            }
            plugin_bytes = Some(data.to_vec());
        }
    }

    let bytes = plugin_bytes
        .ok_or_else(|| ApiError::Validation("Missing 'plugin' field in multipart upload".into()))?;

    // Extract and parse jotti-plugin.toml from the archive
    let manifest = extract_manifest(&bytes)
        .map_err(|e| ApiError::Validation(format!("Invalid plugin archive: {}", e)))?;

    let slug = sanitize_slug(&manifest.plugin.slug)
        .map_err(|e| ApiError::Validation(e))?;

    // Write archive to plugin directory
    let install_dir = format!("{}/{}", PLUGIN_INSTALL_DIR, slug);
    tokio::fs::create_dir_all(&install_dir).await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Cannot create plugin dir: {}", e)))?;
    let archive_path = format!("{}/{}", install_dir, filename);
    tokio::fs::write(&archive_path, &bytes).await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Cannot write archive: {}", e)))?;

    let manifest_json = serde_json::to_value(&manifest.events)
        .unwrap_or_default();

    let requires_plan = manifest.plugin.requires_plan
        .as_deref()
        .unwrap_or("community")
        .to_string();

    let plugin_id = Uuid::new_v4();

    sqlx::query!(
        r#"INSERT INTO plugins
               (id, slug, name, version, author, description, tags, homepage,
                enabled, manifest, requires_plan, install_path, installed_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, true, $9::jsonb, $10, $11, NOW())
           ON CONFLICT (slug) DO UPDATE SET
               name        = EXCLUDED.name,
               version     = EXCLUDED.version,
               author      = EXCLUDED.author,
               description = EXCLUDED.description,
               tags        = EXCLUDED.tags,
               manifest    = EXCLUDED.manifest,
               install_path = EXCLUDED.install_path,
               updated_at  = NOW()"#,
        plugin_id,
        slug,
        manifest.plugin.name,
        manifest.plugin.version,
        manifest.plugin.author,
        manifest.plugin.description,
        &manifest.plugin.tags,
        manifest.plugin.homepage,
        manifest_json,
        requires_plan,
        archive_path,
    )
    .execute(&state.db)
    .await?;

    // Register event subscriptions
    for (event_name, handler) in &manifest.events {
        sqlx::query!(
            "INSERT INTO plugin_events (plugin_id, event_name, handler)
             VALUES ((SELECT id FROM plugins WHERE slug = $1), $2, $3)
             ON CONFLICT (plugin_id, event_name) DO UPDATE SET handler = EXCLUDED.handler",
            slug, event_name, handler
        )
        .execute(&state.db)
        .await?;
    }

    Ok((StatusCode::CREATED, Json(PluginResponse {
        slug:         slug.clone(),
        name:         manifest.plugin.name,
        version:      manifest.plugin.version,
        author:       manifest.plugin.author,
        description:  manifest.plugin.description,
        tags:         manifest.plugin.tags,
        homepage:     manifest.plugin.homepage,
        icon_url:     None,
        installed:    true,
        enabled:      true,
        manifest:     manifest_json,
        requires_plan,
        installed_at: Utc::now(),
    })))
}

/// Install a built-in/marketplace plugin by slug (no upload needed).
async fn install_plugin(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(slug): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    require_admin(&claims)?;

    let slug = sanitize_slug(&slug)
        .map_err(|e| ApiError::Validation(e))?;

    // Idempotent — if already installed, just return success
    let existing = sqlx::query_scalar!(
        "SELECT id FROM plugins WHERE slug = $1", slug
    )
    .fetch_optional(&state.db)
    .await?;

    if existing.is_some() {
        return Ok(Json(serde_json::json!({
            "slug": slug,
            "message": "Plugin already installed",
        })));
    }

    // Marketplace plugins are downloaded from the JottiCP registry in prod.
    // In this stub, we insert a placeholder record so the UI can toggle/remove it.
    let plugin_id = Uuid::new_v4();
    sqlx::query!(
        r#"INSERT INTO plugins (id, slug, name, version, author, description, enabled)
           VALUES ($1, $2, $3, '1.0.0', 'JottiCP Team', 'Marketplace plugin', true)
           ON CONFLICT (slug) DO NOTHING"#,
        plugin_id, slug, slug,
    )
    .execute(&state.db)
    .await?;

    Ok(Json(serde_json::json!({
        "slug":    slug,
        "message": "Plugin installed",
    })))
}

async fn enable_plugin(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(slug): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    require_admin(&claims)?;
    let slug = sanitize_slug(&slug).map_err(|e| ApiError::Validation(e))?;

    let updated = sqlx::query!(
        "UPDATE plugins SET enabled = true, updated_at = NOW() WHERE slug = $1",
        slug
    )
    .execute(&state.db)
    .await?;

    if updated.rows_affected() == 0 {
        return Err(ApiError::NotFound { resource: "plugin" });
    }
    Ok(Json(serde_json::json!({ "slug": slug, "enabled": true })))
}

async fn disable_plugin(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(slug): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    require_admin(&claims)?;
    let slug = sanitize_slug(&slug).map_err(|e| ApiError::Validation(e))?;

    let updated = sqlx::query!(
        "UPDATE plugins SET enabled = false, updated_at = NOW() WHERE slug = $1",
        slug
    )
    .execute(&state.db)
    .await?;

    if updated.rows_affected() == 0 {
        return Err(ApiError::NotFound { resource: "plugin" });
    }
    Ok(Json(serde_json::json!({ "slug": slug, "enabled": false })))
}

async fn uninstall_plugin(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    require_admin(&claims)?;
    let slug = sanitize_slug(&slug).map_err(|e| ApiError::Validation(e))?;

    // Get install path before deletion so we can remove files
    let row = sqlx::query!(
        "DELETE FROM plugins WHERE slug = $1 RETURNING install_path",
        slug
    )
    .fetch_optional(&state.db)
    .await?;

    if row.is_none() {
        return Err(ApiError::NotFound { resource: "plugin" });
    }

    // Best-effort file removal
    if let Some(Some(path)) = row.map(|r| r.install_path) {
        let _ = tokio::fs::remove_dir_all(&path).await;
    }

    Ok(StatusCode::NO_CONTENT)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn sanitize_slug(slug: &str) -> Result<String, String> {
    let s = slug.trim().to_lowercase();
    if s.is_empty() || s.len() > 64 {
        return Err("Plugin slug must be 1–64 characters".into());
    }
    if !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("Plugin slug must contain only letters, digits, hyphens, and underscores".into());
    }
    // Prevent path traversal
    if s.contains("..") || s.starts_with('-') {
        return Err("Invalid plugin slug".into());
    }
    Ok(s)
}

/// Extract `jotti-plugin.toml` from a gzipped tar archive and parse it.
fn extract_manifest(data: &[u8]) -> anyhow::Result<PluginManifest> {
    use std::io::Read;
    let gz = flate2::read::GzDecoder::new(std::io::Cursor::new(data));
    let mut archive = tar::Archive::new(gz);

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_owned();
        let filename = path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if filename == "jotti-plugin.toml" {
            let mut contents = String::new();
            entry.read_to_string(&mut contents)?;
            let manifest: PluginManifest = toml::from_str(&contents)
                .map_err(|e| anyhow::anyhow!("jotti-plugin.toml parse error: {}", e))?;
            return Ok(manifest);
        }
    }
    Err(anyhow::anyhow!("jotti-plugin.toml not found in archive root"))
}
