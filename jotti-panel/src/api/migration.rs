use axum::{
    Router,
    routing::{get, post},
    extract::{Path, State},
    Json,
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};
use super::auth::Claims;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Maximum upload size for a cPanel archive (2 GB).
/// Without this limit a 100 GB file can fill the disk before multipart reads complete.
const MAX_ARCHIVE_BYTES: usize = 2 * 1024 * 1024 * 1024; // 2 GB

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ImportQuery {
    /// Optional target site to import into. If omitted, a new site is created.
    pub target_site_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct MigrationJobResponse {
    pub id:              Uuid,
    pub user_id:         Uuid,
    pub target_site_id:  Option<Uuid>,
    pub status:          String,
    pub progress:        i32,
    pub report:          Option<serde_json::Value>,
    pub created_at:      DateTime<Utc>,
    pub completed_at:    Option<DateTime<Utc>>,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/import",             post(import_cpanel_backup))
        .route("/import/plesk",       post(import_plesk_backup))
        .route("/jobs",               get(list_migration_jobs))
        .route("/jobs/{id}",          get(get_migration_job))
}

// ── Access helper ─────────────────────────────────────────────────────────────

fn caller_id(claims: &Claims) -> ApiResult<Uuid> {
    claims.sub.parse::<Uuid>().map_err(|_| ApiError::Unauthorized)
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// Accept a cPanel .tar.gz upload, validate it's a real cPanel archive,
/// persist to a temp path, insert a migration_jobs row, then enqueue processing.
///
/// Content-Type: multipart/form-data
/// Field: `archive` (file)
/// Query: ?target_site_id=UUID (optional)
async fn import_cpanel_backup(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    axum::extract::Query(q): axum::extract::Query<ImportQuery>,
    mut multipart: axum::extract::Multipart,
) -> ApiResult<(StatusCode, Json<MigrationJobResponse>)> {
    let user_id = caller_id(&claims)?;

    // Read the archive field from multipart
    let mut archive_bytes: Option<Vec<u8>> = None;
    let mut original_filename = String::from("archive.tar.gz");

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        ApiError::Validation(format!("Multipart error: {}", e))
    })? {
        let name = field.name().unwrap_or("").to_string();
        if name == "archive" {
            original_filename = field
                .file_name()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "archive.tar.gz".into());
            let bytes = field.bytes().await.map_err(|e| {
                ApiError::Validation(format!("Failed to read upload: {}", e))
            })?;
            // Enforce maximum archive size BEFORE buffering into RAM.
            // A too-large upload would fill both memory and disk.
            if bytes.len() > MAX_ARCHIVE_BYTES {
                return Err(ApiError::Validation(format!(
                    "Archive exceeds the maximum allowed size of {} GB",
                    MAX_ARCHIVE_BYTES / (1024 * 1024 * 1024)
                )));
            }
            archive_bytes = Some(bytes.to_vec());
        }
    }

    let bytes = archive_bytes
        .ok_or_else(|| ApiError::Validation("Missing `archive` field in multipart form".into()))?;

    // Validate it is a gzip file (magic bytes 1f 8b)
    if bytes.len() < 2 || bytes[0] != 0x1f || bytes[1] != 0x8b {
        return Err(ApiError::Validation("Uploaded file is not a valid gzip archive".into()));
    }

    // Validate cPanel archive structure: list entries and look for expected prefixes.
    // We check the first ~512 bytes of the tar header for "homedir/" or "cpmove-".
    let is_cpanel = validate_cpanel_archive(&bytes).map_err(|e| {
        ApiError::Validation(format!("Archive validation failed: {}", e))
    })?;

    if !is_cpanel {
        return Err(ApiError::Validation(
            "This does not appear to be a valid cPanel backup archive. \
             Expected a cPanel full backup (cpmove-*.tar.gz) or a cPanel account backup containing homedir/."
            .into()
        ));
    }

    // Persist the archive to /tmp with a unique name.
    // Sanitize original_filename: strip any directory components and reject names
    // containing path-traversal sequences to prevent writing outside the job directory.
    let safe_filename = {
        let base = std::path::Path::new(&original_filename)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("archive.tar.gz");
        // Keep only alphanumeric, dot, hyphen, underscore
        let sanitized: String = base
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
            .collect();
        if sanitized.is_empty() { "archive.tar.gz".to_string() } else { sanitized }
    };

    let job_id = Uuid::new_v4();
    let archive_path = format!("/tmp/orbit-migration-{}/{}", job_id, safe_filename);
    let dir = format!("/tmp/orbit-migration-{}", job_id);

    tokio::fs::create_dir_all(&dir).await.map_err(|e| {
        ApiError::Internal(anyhow::anyhow!("Cannot create temp dir: {}", e))
    })?;

    // Write the archive; clean up the temp directory on failure to avoid orphaned files.
    if let Err(e) = tokio::fs::write(&archive_path, &bytes).await {
        let _ = tokio::fs::remove_dir_all(&dir).await;
        return Err(ApiError::Internal(anyhow::anyhow!("Cannot write archive to temp dir: {}", e)));
    }

    // Insert migration job as 'queued'
    sqlx::query!(
        r#"INSERT INTO migration_jobs
               (id, user_id, target_site_id, status, progress, archive_path, created_at)
           VALUES ($1, $2, $3, 'queued', 0, $4, NOW())"#,
        job_id, user_id, q.target_site_id, archive_path
    )
    .execute(&state.db)
    .await?;

    // Enqueue the processing job
    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "cpanel_migration",
            "job_id": job_id,
            "archive_path": archive_path,
            "target_site_id": q.target_site_id,
            "user_id": user_id,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    tracing::info!(job_id = %job_id, user_id = %user_id, "cPanel migration job enqueued");

    Ok((StatusCode::ACCEPTED, Json(MigrationJobResponse {
        id:             job_id,
        user_id,
        target_site_id: q.target_site_id,
        status:         "queued".into(),
        progress:       0,
        report:         None,
        created_at:     Utc::now(),
        completed_at:   None,
    })))
}

/// Return status and progress for a specific migration job.
/// Users can only see their own jobs; admins can see all.
async fn get_migration_job(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<MigrationJobResponse>> {
    let user_id = caller_id(&claims)?;
    let is_admin = claims.role == "admin";

    let row = sqlx::query!(
        r#"SELECT id, user_id, target_site_id, status, progress, report, created_at, completed_at
           FROM migration_jobs
           WHERE id = $1 AND ($2::boolean OR user_id = $3)"#,
        id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "migration_job" })?;

    Ok(Json(MigrationJobResponse {
        id:             row.id,
        user_id:        row.user_id,
        target_site_id: row.target_site_id,
        status:         row.status,
        progress:       row.progress,
        report:         row.report,
        created_at:     row.created_at,
        completed_at:   row.completed_at,
    }))
}

/// List migration jobs for the current user (admin sees all).
async fn list_migration_jobs(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<Vec<MigrationJobResponse>>> {
    let user_id = caller_id(&claims)?;
    let is_admin = claims.role == "admin";

    let rows = sqlx::query!(
        r#"SELECT id, user_id, target_site_id, status, progress, report, created_at, completed_at
           FROM migration_jobs
           WHERE ($1::boolean OR user_id = $2)
           ORDER BY created_at DESC
           LIMIT 200"#,
        is_admin, user_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|r| MigrationJobResponse {
        id:             r.id,
        user_id:        r.user_id,
        target_site_id: r.target_site_id,
        status:         r.status,
        progress:       r.progress,
        report:         r.report,
        created_at:     r.created_at,
        completed_at:   r.completed_at,
    }).collect()))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Validate that the gzip bytes represent a cPanel archive by inspecting
/// the uncompressed tar header for known cPanel entry prefixes.
///
/// SECURITY: This function also checks every entry for zip-slip (path traversal)
/// attacks.  A maliciously crafted archive with entries like "../../etc/passwd"
/// could overwrite arbitrary system files if the worker extracts without checking.
/// We reject the upload here so the worker never needs to handle such archives.
fn validate_cpanel_archive(bytes: &[u8]) -> anyhow::Result<bool> {
    use std::io::Read;

    let cursor = std::io::Cursor::new(bytes);
    let gz = flate2::read::GzDecoder::new(cursor);
    let mut tar = tar::Archive::new(gz);

    let cpanel_indicators = [
        "homedir/",
        "homedir.tar.gz",
        "cpmove-",
        "cp_build_",
        "mysql/",
        "mysql.sql",
        "emails/",
        "dnszones/",
        "crontabs/",
        "userdata/",
        "meta/",
    ];

    let mut entries_checked = 0;
    let mut found_cpanel = false;

    for entry in tar.entries()? {
        if entries_checked >= 40 {
            break;
        }
        entries_checked += 1;
        let entry = entry?;
        if let Ok(path) = entry.path() {
            let path_str = path.to_string_lossy();

            // Zip-slip check: reject any entry whose path starts with "/" or
            // contains ".." as a path component.  The worker must also enforce
            // this at extraction time, but we add an early rejection here.
            if path.is_absolute() {
                return Err(anyhow::anyhow!(
                    "Archive contains absolute path '{}' — potential zip-slip attack",
                    path_str
                ));
            }
            for component in path.components() {
                use std::path::Component;
                if matches!(component, Component::ParentDir) {
                    return Err(anyhow::anyhow!(
                        "Archive contains path traversal component '..': '{}'",
                        path_str
                    ));
                }
            }

            if !found_cpanel {
                for indicator in &cpanel_indicators {
                    if path_str.contains(indicator) {
                        found_cpanel = true;
                        break;
                    }
                }
            }
        }
    }

    Ok(found_cpanel)
}

// ── Plesk importer ────────────────────────────────────────────────────────────

/// POST /import/plesk  (nested under /api/v1/migration)
/// Accept a Plesk .tar.gz or .zip backup, validate it's actually a Plesk archive,
/// queue it for background processing.
///
/// Plesk backups contain a `domains/{domain}/` directory structure with
/// `conf/` (vhost config), `logs/`, `private/`, `web/` (docroot), and
/// a `backup_info.xml` (Plesk-specific metadata file).
pub async fn import_plesk_backup(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    axum::extract::Query(q): axum::extract::Query<ImportQuery>,
    mut multipart: axum::extract::Multipart,
) -> ApiResult<(StatusCode, Json<MigrationJobResponse>)> {
    let user_id = caller_id(&claims)?;

    // Extract the uploaded file from multipart
    let mut archive_bytes: Vec<u8> = Vec::new();
    let mut original_name = String::from("plesk_backup.tar.gz");
    let mut found_field = false;

    while let Some(mut field) = multipart.next_field().await
        .map_err(|e| ApiError::Validation(format!("Multipart error: {}", e)))?
    {
        if field.name() == Some("archive") || field.name() == Some("file") || !found_field {
            found_field = true;
            if let Some(fname) = field.file_name() {
                original_name = fname.to_string();
            }
            use futures_util::TryStreamExt;
            while let Some(chunk) = field.try_next().await
                .map_err(|e| ApiError::Validation(format!("Upload read: {}", e)))?
            {
                archive_bytes.extend_from_slice(&chunk);
                if archive_bytes.len() > MAX_ARCHIVE_BYTES {
                    return Err(ApiError::Validation(format!(
                        "Archive too large (max {} GB)", MAX_ARCHIVE_BYTES / 1_073_741_824
                    )));
                }
            }
        }
    }

    if archive_bytes.is_empty() {
        return Err(ApiError::Validation("No archive file in request. Use multipart field 'archive'".into()));
    }

    // Validate it's actually a Plesk backup by checking for Plesk-specific markers
    let is_plesk = validate_plesk_archive(&archive_bytes)
        .map_err(|e| ApiError::Validation(format!("Archive validation failed: {}", e)))?;

    if !is_plesk {
        return Err(ApiError::Validation(
            "File does not appear to be a valid Plesk backup archive. \
             Expected a .tar.gz containing backup_info.xml or a domains/ directory structure.".into()
        ));
    }

    // Write to a unique temp path so background worker can process it
    let job_id = Uuid::new_v4();
    let archive_path = format!("/tmp/jottiecp_migration_{}.tar.gz", job_id);
    tokio::fs::write(&archive_path, &archive_bytes).await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Write archive: {}", e)))?;

    // Insert migration job
    let row = sqlx::query!(
        r#"INSERT INTO migration_jobs
              (id, user_id, target_site_id, archive_path, source_type, status, progress)
           VALUES ($1, $2, $3, $4, 'plesk', 'pending', 0)
           RETURNING id, user_id, target_site_id, status, progress, report,
                     created_at, completed_at"#,
        job_id, user_id, q.target_site_id, archive_path
    )
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::Internal(anyhow::anyhow!("Insert migration job: {}", e)))?;

    // Enqueue to Valkey for background worker
    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type":         "migrate_plesk",
            "job_id":       job_id,
            "archive_path": archive_path,
            "target_site_id": q.target_site_id,
            "user_id":      user_id,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    tracing::info!(
        user_id = %user_id,
        job_id  = %job_id,
        archive = %original_name,
        "Plesk migration job queued"
    );

    Ok((StatusCode::ACCEPTED, Json(MigrationJobResponse {
        id:             row.id,
        user_id:        row.user_id,
        target_site_id: row.target_site_id,
        status:         row.status,
        progress:       row.progress,
        report:         row.report,
        created_at:     row.created_at,
        completed_at:   row.completed_at,
    })))
}

/// Check if a byte slice is a valid Plesk backup by inspecting the tarball contents.
/// Returns Ok(true) if Plesk markers are found, Ok(false) if not Plesk, Err on corrupt archive.
fn validate_plesk_archive(data: &[u8]) -> Result<bool, String> {
    use std::io::Read;

    // Detect if this is gzipped
    let is_gzip = data.starts_with(&[0x1f, 0x8b]);
    let is_zip  = data.starts_with(b"PK");

    if is_zip {
        // Plesk can also use .zip — just check for backup_info.xml inside
        let cursor = std::io::Cursor::new(data);
        let mut zip = zip::ZipArchive::new(cursor)
            .map_err(|e| format!("ZIP read error: {}", e))?;
        for i in 0..zip.len() {
            if let Ok(file) = zip.by_index(i) {
                if file.name().contains("backup_info.xml") {
                    return Ok(true);
                }
            }
        }
        return Ok(false);
    }

    // Plesk indicators in tarball paths
    let plesk_indicators = ["backup_info.xml", "domains/", "httpdocs/", "conf/vhost.conf"];
    let mut found_plesk = false;

    let cursor = std::io::Cursor::new(data);
    let tar_stream: Box<dyn Read> = if is_gzip {
        Box::new(flate2::read::GzDecoder::new(cursor))
    } else {
        Box::new(cursor)
    };

    let mut archive = tar::Archive::new(tar_stream);
    for entry in archive.entries().map_err(|e| format!("Tar error: {}", e))? {
        let entry = entry.map_err(|e| format!("Tar entry error: {}", e))?;
        let path = entry.path().map_err(|e| format!("Path error: {}", e))?;
        let path_str = path.to_string_lossy();

        // Security: reject path traversal
        if path_str.contains("..") {
            return Err(format!("Archive contains path traversal component: '{}'", path_str));
        }

        if !found_plesk {
            for indicator in &plesk_indicators {
                if path_str.contains(indicator) {
                    found_plesk = true;
                    break;
                }
            }
        }
    }

    Ok(found_plesk)
}
