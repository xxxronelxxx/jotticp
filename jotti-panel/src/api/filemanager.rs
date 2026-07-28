use axum::{
    Router,
    routing::{delete, get, post},
    extract::{Query, State},
    Json,
    http::{StatusCode, header},
    body::Body,
    response::Response,
};
use axum_extra::extract::Multipart;
use serde::{Deserialize, Serialize};
use std::{path::{Path, PathBuf}, sync::Arc};
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};
use super::auth::Claims;

// ── Types ─────────────────────────────────────────────────────────────────────

const MAX_READ_BYTES: u64 = 1 * 1024 * 1024;    // 1 MB text read limit
const MAX_UPLOAD_BYTES: u64 = 50 * 1024 * 1024;  // 50 MB upload limit

#[derive(Debug, Deserialize)]
pub struct PathQuery {
    pub site_id: Uuid,
    pub path:    Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WriteRequest {
    pub site_id: Uuid,
    pub path:    String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct MkdirRequest {
    pub site_id: Uuid,
    pub path:    String,
}

#[derive(Debug, Deserialize)]
pub struct DeleteRequest {
    pub site_id:   Uuid,
    pub path:      String,
    pub recursive: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct RenameRequest {
    pub site_id: Uuid,
    #[serde(alias = "path")]
    pub src:     String,
    #[serde(alias = "new_path")]
    pub dst:     String,
}

#[derive(Debug, Deserialize)]
pub struct ChmodRequest {
    pub site_id: Uuid,
    pub path:    String,
    pub mode:    String, // octal string "0644"
}

#[derive(Debug, Deserialize)]
pub struct CompressRequest {
    pub site_id: Uuid,
    pub paths:   Vec<String>,
    pub output:  String,
    pub format:  String, // "tar.gz" | "zip"
}

#[derive(Debug, Deserialize)]
pub struct ExtractRequest {
    pub site_id:    Uuid,
    pub archive:    String,
    pub dest:       Option<String>, // destination directory, defaults to archive's directory
}

#[derive(Debug, Serialize)]
pub struct DirEntry {
    pub name:         String,
    pub path:         String,
    pub is_dir:       bool,
    pub size:         u64,
    pub modified_at:  String,  // ISO 8601
    pub permissions:  String,
    pub owner:        String,
}

#[derive(Debug, Serialize)]
pub struct ListDirResponse {
    pub entries: Vec<DirEntry>,
    pub path:    String,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/files",          get(list_dir).delete(delete_file))
        .route("/api/v1/files/read",     get(read_file))
        .route("/api/v1/files/write",    post(write_file))
        .route("/api/v1/files/upload",   post(upload_file))
        .route("/api/v1/files/download", get(download_file))
        .route("/api/v1/files/mkdir",    post(mkdir))
        .route("/api/v1/files/rename",   post(rename))
        .route("/api/v1/files/chmod",    post(chmod))
        .route("/api/v1/files/compress", post(compress))
        .route("/api/v1/files/extract",  post(extract))
}

// ── Access helper ─────────────────────────────────────────────────────────────

fn caller(claims: &Claims) -> ApiResult<(Uuid, bool)> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    let is_admin = claims.role == "admin";
    Ok((user_id, is_admin))
}

/// Fetch the site's unix_user, verify ownership, build and return the chroot base path.
/// The returned path is canonicalized so that symlink-based traversal is caught by starts_with checks.
async fn site_home(state: &AppState, site_id: Uuid, user_id: Uuid, is_admin: bool) -> ApiResult<PathBuf> {
    let site = sqlx::query!(
        "SELECT unix_user FROM sites WHERE id = $1 AND deleted_at IS NULL AND ($2::boolean OR owner_id = $3)",
        site_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "site" })?;

    let raw = PathBuf::from(format!("/home/{}", site.unix_user));
    // Canonicalize so that symlinks in the base path itself are resolved before comparison
    let canonical = std::fs::canonicalize(&raw)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Cannot resolve site home: {}", e)))?;
    Ok(canonical)
}

/// Resolve a user-supplied relative path inside `base`, verifying it does not escape.
/// Returns the canonicalized absolute path.
fn safe_resolve(base: &Path, user_path: &str) -> ApiResult<PathBuf> {
    // Strip leading slashes so it can't be treated as absolute
    let stripped = user_path.trim_start_matches('/');
    let candidate = base.join(stripped);

    // Resolve all symlinks and ./ ../
    let resolved = std::fs::canonicalize(&candidate).unwrap_or(candidate.clone());

    // Ensure the result is inside base
    if !resolved.starts_with(base) {
        return Err(ApiError::Forbidden("Path traversal detected".into()));
    }
    Ok(resolved)
}

/// Same as safe_resolve but doesn't require the path to already exist (for write/mkdir).
fn safe_resolve_new(base: &Path, user_path: &str) -> ApiResult<PathBuf> {
    let stripped = user_path.trim_start_matches('/');
    let candidate = base.join(stripped);

    // Normalize without requiring existence
    let normalized = normalize_path(&candidate);

    if !normalized.starts_with(base) {
        return Err(ApiError::Forbidden("Path traversal detected".into()));
    }
    Ok(normalized)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => { out.pop(); }
            std::path::Component::CurDir    => {}
            c                               => out.push(c),
        }
    }
    out
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn list_dir(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Query(q): Query<PathQuery>,
) -> ApiResult<Json<ListDirResponse>> {
    let (user_id, is_admin) = caller(&claims)?;
    let base = site_home(&state, q.site_id, user_id, is_admin).await?;

    let dir_path = match q.path.as_deref() {
        Some(p) if !p.is_empty() => safe_resolve(&base, p)?,
        _ => base.clone(),
    };

    let current_path = match q.path.as_deref() {
        Some(p) if !p.is_empty() => p.to_string(),
        _ => "/".to_string(),
    };

    let mut entries: Vec<DirEntry> = Vec::new();

    let mut read_dir = tokio::fs::read_dir(&dir_path).await
        .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound {
            ApiError::NotFound { resource: "directory" }
        } else {
            ApiError::Internal(anyhow::anyhow!("Cannot read directory: {}", e))
        })?;

    while let Some(entry) = read_dir.next_entry().await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Directory read error: {}", e)))?
    {
        let meta = entry.metadata().await.ok();
        let file_type = entry.file_type().await.ok();
        let is_dir = file_type.map(|t| t.is_dir()).unwrap_or(false);
        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let modified_at = meta.as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| {
                use chrono::TimeZone;
                chrono::Utc.timestamp_opt(d.as_secs() as i64, 0)
                    .single()
                    .map(|dt| dt.to_rfc3339())
                    .unwrap_or_default()
            })
            .unwrap_or_default();

        #[cfg(unix)]
        let permissions = {
            use std::os::unix::fs::MetadataExt;
            let mode = meta.as_ref().map(|m| m.mode()).unwrap_or(0);
            format!("{:04o}", mode & 0o7777)
        };
        #[cfg(not(unix))]
        let permissions = "0644".to_string();

        let rel_path = entry.path()
            .strip_prefix(&base)
            .unwrap_or(entry.path().as_path())
            .to_string_lossy()
            .to_string();

        entries.push(DirEntry {
            name:        entry.file_name().to_string_lossy().to_string(),
            path:        format!("/{}", rel_path),
            is_dir,
            size,
            modified_at,
            permissions,
            owner:       String::new(),
        });
    }

    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    Ok(Json(ListDirResponse { entries, path: current_path }))
}

async fn read_file(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Query(q): Query<PathQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let (user_id, is_admin) = caller(&claims)?;
    let base = site_home(&state, q.site_id, user_id, is_admin).await?;

    let path_str = q.path.as_deref().unwrap_or("");
    let file_path = safe_resolve(&base, path_str)?;

    let meta = tokio::fs::metadata(&file_path).await
        .map_err(|_| ApiError::NotFound { resource: "file" })?;

    if meta.is_dir() {
        return Err(ApiError::Validation("Path is a directory, use list endpoint".into()));
    }

    if meta.len() > MAX_READ_BYTES {
        return Err(ApiError::Validation(format!(
            "File too large to read inline ({}MB > 1MB). Use the download endpoint.",
            meta.len() / 1024 / 1024
        )));
    }

    let content = tokio::fs::read_to_string(&file_path).await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Read error: {}", e)))?;

    Ok(Json(serde_json::json!({
        "path": path_str,
        "content": content,
        "size": meta.len(),
    })))
}

async fn write_file(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<WriteRequest>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;
    let base = site_home(&state, req.site_id, user_id, is_admin).await?;
    let file_path = safe_resolve_new(&base, &req.path)?;

    // Ensure parent directory exists
    if let Some(parent) = file_path.parent() {
        tokio::fs::create_dir_all(parent).await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("Cannot create parent dirs: {}", e)))?;
    }

    tokio::fs::write(&file_path, req.content.as_bytes()).await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Write error: {}", e)))?;

    // Ensure written file is not executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o644);
        tokio::fs::set_permissions(&file_path, perms).await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("chmod after write failed: {}", e)))?;
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn upload_file(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    mut multipart: Multipart,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    // We need site_id from a form field before we can validate ownership
    let mut site_id: Option<Uuid> = None;
    let mut dest_path: Option<String> = None;
    let mut file_data: Option<(String, Vec<u8>)> = None;

    while let Some(field) = multipart.next_field().await
        .map_err(|e| ApiError::Validation(format!("Multipart error: {}", e)))?
    {
        let field_name = field.name().unwrap_or("").to_string();
        match field_name.as_str() {
            "site_id" => {
                let v = field.text().await.map_err(|_| ApiError::Validation("Invalid site_id".into()))?;
                site_id = v.parse().ok();
            }
            "path" => {
                dest_path = Some(field.text().await.map_err(|_| ApiError::Validation("Invalid path".into()))?);
            }
            "file" => {
                let filename = field.file_name().unwrap_or("upload").to_string();
                let data = field.bytes().await.map_err(|e| ApiError::Validation(format!("Upload error: {}", e)))?;
                if data.len() as u64 > MAX_UPLOAD_BYTES {
                    return Err(ApiError::Validation("File exceeds 50MB upload limit".into()));
                }
                file_data = Some((filename, data.to_vec()));
            }
            _ => {}
        }
    }

    let site_id  = site_id.ok_or_else(|| ApiError::Validation("site_id field required".into()))?;
    let (filename, data) = file_data.ok_or_else(|| ApiError::Validation("file field required".into()))?;

    let (user_id, is_admin) = caller(&claims)?;
    let base = site_home(&state, site_id, user_id, is_admin).await?;

    let dest_dir = match dest_path.as_deref() {
        Some(p) if !p.is_empty() => safe_resolve_new(&base, p)?,
        _ => base.clone(),
    };

    // Reject filenames containing path separators or traversal — the destination
    // directory has already been resolved + validated, we just append the leaf name.
    if filename.contains('/') || filename.contains('\\') || filename == ".." || filename == "." {
        return Err(ApiError::Validation("Invalid filename".into()));
    }
    let file_path = dest_dir.join(&filename);
    let normalized = normalize_path(&file_path);
    if !normalized.starts_with(&base) {
        return Err(ApiError::Forbidden("Path traversal detected".into()));
    }
    let file_path = normalized;

    tokio::fs::create_dir_all(&dest_dir).await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Cannot create dest dir: {}", e)))?;

    tokio::fs::write(&file_path, &data).await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Upload write error: {}", e)))?;

    // Force non-executable permissions on uploaded file
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o644);
        tokio::fs::set_permissions(&file_path, perms).await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("chmod after upload failed: {}", e)))?;
    }

    let rel = file_path.strip_prefix(&base).unwrap_or(&file_path);
    Ok((StatusCode::CREATED, Json(serde_json::json!({
        "path": format!("/{}", rel.to_string_lossy()),
        "size": data.len(),
        "filename": filename,
    }))))
}

async fn download_file(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Query(q): Query<PathQuery>,
) -> ApiResult<Response<Body>> {
    let (user_id, is_admin) = caller(&claims)?;
    let base = site_home(&state, q.site_id, user_id, is_admin).await?;

    let path_str = q.path.as_deref().unwrap_or("");
    let file_path = safe_resolve(&base, path_str)?;

    let meta = tokio::fs::metadata(&file_path).await
        .map_err(|_| ApiError::NotFound { resource: "file" })?;

    if meta.is_dir() {
        return Err(ApiError::Validation("Cannot download a directory directly — use compress first".into()));
    }

    let file = tokio::fs::File::open(&file_path).await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Cannot open file: {}", e)))?;

    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);

    // Sanitize filename for the Content-Disposition header — strip any character
    // that's invalid in HTTP header values (quotes, CR, LF, control chars).
    let raw_name = file_path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("download");
    let safe_name: String = raw_name.chars()
        .filter(|c| !c.is_control() && *c != '"' && *c != '\\')
        .collect();
    let safe_name = if safe_name.is_empty() { "download".to_string() } else { safe_name };
    let disposition = format!("attachment; filename=\"{}\"", safe_name);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_DISPOSITION, disposition)
        .header(header::CONTENT_LENGTH, meta.len().to_string())
        .body(body)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Failed to build response: {}", e)))
}

async fn mkdir(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<MkdirRequest>,
) -> ApiResult<StatusCode> {
    if req.path.trim().is_empty() || req.path.trim() == "/" {
        return Err(ApiError::Validation("path is required and cannot be empty or root".into()));
    }
    let (user_id, is_admin) = caller(&claims)?;
    let base = site_home(&state, req.site_id, user_id, is_admin).await?;
    let dir_path = safe_resolve_new(&base, &req.path)?;

    tokio::fs::create_dir_all(&dir_path).await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Cannot create directory: {}", e)))?;

    Ok(StatusCode::NO_CONTENT)
}

async fn delete_file(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(q): Json<DeleteRequest>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;
    let base = site_home(&state, q.site_id, user_id, is_admin).await?;
    let target = safe_resolve(&base, &q.path)?;

    // Do not allow deleting the site home root
    if target == base {
        return Err(ApiError::Forbidden("Cannot delete site root directory".into()));
    }

    let meta = tokio::fs::metadata(&target).await
        .map_err(|_| ApiError::NotFound { resource: "file" })?;

    if meta.is_dir() {
        if q.recursive.unwrap_or(false) {
            tokio::fs::remove_dir_all(&target).await
                .map_err(|e| ApiError::Internal(anyhow::anyhow!("Cannot remove directory: {}", e)))?;
        } else {
            tokio::fs::remove_dir(&target).await
                .map_err(|e| ApiError::Validation(format!("Directory not empty. Use recursive=true. Error: {}", e)))?;
        }
    } else {
        tokio::fs::remove_file(&target).await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("Cannot delete file: {}", e)))?;
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn rename(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<RenameRequest>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;
    let base = site_home(&state, req.site_id, user_id, is_admin).await?;

    let src = safe_resolve(&base, &req.src)?;
    let dst = safe_resolve_new(&base, &req.dst)?;

    if let Some(parent) = dst.parent() {
        if !parent.exists() {
            tokio::fs::create_dir_all(parent).await
                .map_err(|e| ApiError::Internal(anyhow::anyhow!("Cannot create parent dir: {}", e)))?;
        }
    }

    tokio::fs::rename(&src, &dst).await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Rename failed: {}", e)))?;

    Ok(StatusCode::NO_CONTENT)
}

async fn chmod(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<ChmodRequest>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;
    let base = site_home(&state, req.site_id, user_id, is_admin).await?;
    let target = safe_resolve(&base, &req.path)?;

    // Parse octal mode string
    let mode = u32::from_str_radix(req.mode.trim_start_matches('0'), 8)
        .map_err(|_| ApiError::Validation(format!("Invalid octal mode: {}", req.mode)))?;

    // Restrict to safe permission bits only (prevent setuid/setgid)
    if mode > 0o777 {
        return Err(ApiError::Validation("Permission mode must be <= 0777 (no setuid/setgid)".into()));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(mode);
        tokio::fs::set_permissions(&target, perms).await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("chmod failed: {}", e)))?;
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn compress(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<CompressRequest>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let (user_id, is_admin) = caller(&claims)?;
    let base = site_home(&state, req.site_id, user_id, is_admin).await?;

    let output_path = safe_resolve_new(&base, &req.output)?;

    // Validate all source paths
    let mut sources = Vec::new();
    for p in &req.paths {
        sources.push(safe_resolve(&base, p)?);
    }

    if sources.is_empty() {
        return Err(ApiError::Validation("No paths provided to compress".into()));
    }

    match req.format.as_str() {
        "tar.gz" => compress_targz(&sources, &output_path)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("Compression failed: {}", e)))?,
        "zip" => compress_zip(&sources, &output_path, &base)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("Zip failed: {}", e)))?,
        _ => return Err(ApiError::Validation("format must be 'tar.gz' or 'zip'".into())),
    }

    let size = std::fs::metadata(&output_path).map(|m| m.len()).unwrap_or(0);
    let rel = output_path.strip_prefix(&base).unwrap_or(&output_path);

    Ok((StatusCode::CREATED, Json(serde_json::json!({
        "path": format!("/{}", rel.to_string_lossy()),
        "size": size,
        "format": req.format,
    }))))
}

fn compress_targz(sources: &[PathBuf], output: &Path) -> anyhow::Result<()> {
    let file = std::fs::File::create(output)?;
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);

    for src in sources {
        if src.is_dir() {
            builder.append_dir_all(
                src.file_name().unwrap_or_default(),
                src,
            )?;
        } else {
            builder.append_path_with_name(src, src.file_name().unwrap_or_default())?;
        }
    }

    builder.finish()?;
    Ok(())
}

fn compress_zip(sources: &[PathBuf], output: &Path, base: &Path) -> anyhow::Result<()> {
    let file = std::fs::File::create(output)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::FileOptions::<()>::default()
        .compression_method(zip::CompressionMethod::Deflated);

    for src in sources {
        if src.is_dir() {
            for entry in walkdir::WalkDir::new(src).into_iter().flatten() {
                let path = entry.path();
                let name = path.strip_prefix(base).unwrap_or(path).to_string_lossy().to_string();
                if path.is_dir() {
                    zip.add_directory(&name, options)?;
                } else {
                    zip.start_file(&name, options)?;
                    let mut f = std::fs::File::open(path)?;
                    std::io::copy(&mut f, &mut zip)?;
                }
            }
        } else {
            let name = src.strip_prefix(base).unwrap_or(src).to_string_lossy().to_string();
            zip.start_file(&name, options)?;
            let mut f = std::fs::File::open(src)?;
            std::io::copy(&mut f, &mut zip)?;
        }
    }

    zip.finish()?;
    Ok(())
}

async fn extract(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<ExtractRequest>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let (user_id, is_admin) = caller(&claims)?;
    let base = site_home(&state, req.site_id, user_id, is_admin).await?;

    let archive_path = safe_resolve(&base, &req.archive)?;

    let dest_path = match req.dest.as_deref() {
        Some(d) if !d.is_empty() => safe_resolve_new(&base, d)?,
        _ => archive_path.parent().unwrap_or(&base).to_path_buf(),
    };

    std::fs::create_dir_all(&dest_path)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Cannot create dest dir: {}", e)))?;

    let archive_name = archive_path.to_string_lossy().to_lowercase();

    if archive_name.ends_with(".tar.gz") || archive_name.ends_with(".tgz") {
        extract_targz(&archive_path, &dest_path)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("Extraction failed: {}", e)))?;
    } else if archive_name.ends_with(".zip") {
        extract_zip(&archive_path, &dest_path, &base)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("Zip extraction failed: {}", e)))?;
    } else {
        return Err(ApiError::Validation("Unsupported archive format. Supported: .tar.gz, .tgz, .zip".into()));
    }

    let rel = dest_path.strip_prefix(&base).unwrap_or(&dest_path);
    Ok((StatusCode::OK, Json(serde_json::json!({
        "extracted_to": format!("/{}", rel.to_string_lossy()),
    }))))
}

fn extract_targz(archive: &Path, dest: &Path) -> anyhow::Result<()> {
    let file = std::fs::File::open(archive)?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(decoder);
    // Do NOT use tar.unpack(dest) — it does not check for tar-slip (paths like ../../etc/passwd).
    // Iterate entries manually and verify each resolved path stays inside dest.
    for entry in tar.entries()? {
        let mut entry = entry?;
        let entry_path = entry.path()?;

        // Build the destination path and normalize it
        let full_dest = dest.join(&entry_path);
        let normalized = normalize_path(&full_dest);

        // Reject any entry that escapes the destination directory
        if !normalized.starts_with(dest) {
            tracing::warn!(
                entry = %entry_path.display(),
                "Tar-slip detected: skipping entry that would escape extraction directory"
            );
            continue;
        }

        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&normalized)?;
        } else {
            if let Some(parent) = normalized.parent() {
                std::fs::create_dir_all(parent)?;
            }
            entry.unpack(&normalized)?;
            // Force non-executable permissions on extracted files
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&normalized, std::fs::Permissions::from_mode(0o644))?;
            }
        }
    }
    Ok(())
}

fn extract_zip(archive: &Path, dest: &Path, base: &Path) -> anyhow::Result<()> {
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)?;

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;

        // Reject null bytes in entry names
        let entry_name = entry.name();
        if entry_name.contains('\0') {
            continue;
        }

        let entry_path = dest.join(entry_name);

        // Normalize and verify the path stays within base
        let normalized = normalize_path(&entry_path);
        if !normalized.starts_with(base) {
            tracing::warn!(
                entry = %entry_name,
                "Zip-slip detected: skipping entry that would escape extraction directory"
            );
            continue;
        }

        // Use the normalized (safe) path for all I/O
        if entry.is_dir() {
            std::fs::create_dir_all(&normalized)?;
        } else {
            if let Some(parent) = normalized.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out = std::fs::File::create(&normalized)?;
            std::io::copy(&mut entry, &mut out)?;
            // Force non-executable permissions on extracted files
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&normalized, std::fs::Permissions::from_mode(0o644))?;
            }
        }
    }

    Ok(())
}
