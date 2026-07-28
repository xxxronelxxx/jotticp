//! Backup handler: TriggerBackup (called internally; not in proto v1 AgentService).
//!
//! Creates a timestamped backup of site files + database, writes a manifest.json
//! with SHA-256 checksums, and returns the manifest path and total size.

use crate::validation::{validate_db_ident, validate_unix_user};
use chrono::Utc;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;
use tonic::Status;
use tracing::{info, warn};

/// Result of a triggered backup.
#[derive(Debug)]
pub struct BackupResult {
    pub manifest_path: String,
    pub total_bytes:   u64,
}

/// Backup manifest written to manifest.json.
#[derive(Debug, serde::Serialize)]
pub struct BackupManifest {
    pub site_id:      String,
    pub domain:       String,
    pub unix_user:    String,
    pub created_at:   String,
    pub files_path:   String,
    pub db_path:      Option<String>,
    pub files_sha256: String,
    pub db_sha256:    Option<String>,
    pub total_bytes:  u64,
}

/// Trigger a backup for a site.
///
/// - `site_id`   — used as the primary identifier in the manifest
/// - `domain`    — human-readable site domain
/// - `unix_user` — site Unix user (files are in /home/<unix_user>/)
/// - `db_name`   — optional MySQL/MariaDB database name (empty = skip DB backup)
pub fn trigger_backup(
    site_id: &str,
    domain: &str,
    unix_user: &str,
    db_name: Option<&str>,
) -> Result<BackupResult, Status> {
    let unix_user = validate_unix_user(unix_user)?;
    let db_name   = db_name
        .filter(|s| !s.is_empty())
        .map(|s| validate_db_ident(s))
        .transpose()?;

    let now           = Utc::now();
    let date_str      = now.format("%Y-%m-%d").to_string();
    let ts_str        = now.format("%Y%m%d%H%M%S").to_string();
    let backup_dir    = format!("/var/backups/jottiecp/{}/{}", site_id, date_str);

    std::fs::create_dir_all(&backup_dir)
        .map_err(|e| Status::internal(format!("create backup dir failed: {}", e)))?;

    // ── 1. Archive site files ──────────────────────────────────────────────
    let files_archive = format!("{}/files-{}.tar.gz", backup_dir, ts_str);
    let site_home     = format!("/home/{}", unix_user);

    if !Path::new(&site_home).exists() {
        return Err(Status::not_found(format!(
            "site home directory {} not found",
            site_home
        )));
    }

    let status = Command::new("/bin/tar")
        .args([
            "-czf",
            &files_archive,
            "-C",
            "/home",
            &unix_user,
            "--exclude=.cache",
            "--exclude=.npm",
            "--exclude=node_modules",
        ])
        .status()
        .map_err(|e| Status::internal(format!("tar exec failed: {}", e)))?;

    if !status.success() {
        return Err(Status::internal(format!(
            "tar failed with exit code {:?}",
            status.code()
        )));
    }

    let files_sha256 = sha256_file(&files_archive)?;
    let files_size   = file_size(&files_archive);

    info!(%domain, archive = %files_archive, size_bytes = files_size, "files backup complete");

    // ── 2. Database backup ─────────────────────────────────────────────────
    let (db_path, db_sha256) = if let Some(db) = db_name.as_deref() {
        let dump_path = format!("{}/db-{}.sql.gz", backup_dir, ts_str);

        let mysqldump = Command::new("/usr/bin/mysqldump")
            .args([
                "--single-transaction",
                "--quick",
                "--lock-tables=false",
                "--default-character-set=utf8mb4",
                db,
            ])
            .output()
            .map_err(|e| Status::internal(format!("mysqldump exec failed: {}", e)))?;

        if !mysqldump.status.success() {
            let stderr = String::from_utf8_lossy(&mysqldump.stderr);
            warn!(%db, stderr = %stderr, "mysqldump failed — skipping DB backup");
            (None, None)
        } else {
            // Pipe dump through gzip
            compress_bytes_to_file(&mysqldump.stdout, &dump_path)
                .map_err(|e| Status::internal(format!("gzip db dump failed: {}", e)))?;

            let sha = sha256_file(&dump_path).unwrap_or_default();
            let size = file_size(&dump_path);
            info!(%db, dump_path = %dump_path, size_bytes = size, "DB backup complete");

            (Some(dump_path), Some(sha))
        }
    } else {
        (None, None)
    };

    // ── 3. Write manifest ──────────────────────────────────────────────────
    let db_size   = db_path.as_deref().map(file_size).unwrap_or(0);
    let total_bytes = files_size + db_size;

    let manifest = BackupManifest {
        site_id:      site_id.to_string(),
        domain:       domain.to_string(),
        unix_user:    unix_user.to_string(),
        created_at:   now.to_rfc3339(),
        files_path:   files_archive,
        db_path:      db_path.clone(),
        files_sha256: files_sha256,
        db_sha256,
        total_bytes,
    };

    let manifest_path = format!("{}/manifest.json", backup_dir);
    let manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| Status::internal(format!("manifest serialise failed: {}", e)))?;

    std::fs::write(&manifest_path, &manifest_json)
        .map_err(|e| Status::internal(format!("write manifest failed: {}", e)))?;

    info!(%domain, manifest_path = %manifest_path, total_bytes, "backup complete");

    Ok(BackupResult {
        manifest_path,
        total_bytes,
    })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn sha256_file(path: &str) -> Result<String, Status> {
    let data = std::fs::read(path)
        .map_err(|e| Status::internal(format!("read file for sha256 failed: {}", e)))?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    Ok(hex::encode(hasher.finalize()))
}

fn file_size(path: &str) -> u64 {
    std::fs::metadata(path)
        .map(|m| m.len())
        .unwrap_or(0)
}

fn compress_bytes_to_file(data: &[u8], dest: &str) -> std::io::Result<()> {
    use flate2::{write::GzEncoder, Compression};
    use std::io::Write;

    let file       = std::fs::File::create(dest)?;
    let mut encoder = GzEncoder::new(file, Compression::default());
    encoder.write_all(data)?;
    encoder.finish()?;
    Ok(())
}
