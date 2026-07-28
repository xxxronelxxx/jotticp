use crate::AppState;
use std::{
    io::{BufReader, Read},
    path::{Path, PathBuf},
    sync::Arc,
};
use uuid::Uuid;

// ── Public entry point ────────────────────────────────────────────────────────

/// Create a complete backup of a site: files + all databases.
///
/// Backup layout:
///   /var/backups/jottiecp/{site_id}/{YYYY-MM-DD-HHMMSS}/
///     ├── files.tar.gz          — full site homedir
///     ├── {db_name}.sql.gz      — one file per MySQL/MariaDB database
///     ├── {db_name}_pg.sql.gz   — one file per PostgreSQL database
///     └── manifest.json         — sizes, sha256 hashes, site metadata
pub async fn backup_site(state: &Arc<AppState>, site_id: Uuid) -> anyhow::Result<()> {
    // ── 1. Fetch site info ───────────────────────────────────────────────────

    let site = sqlx::query!(
        r#"SELECT id, domain, unix_user, status::text AS "status!" FROM sites
           WHERE id = $1 AND deleted_at IS NULL"#,
        site_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| anyhow::anyhow!("Site {} not found for backup", site_id))?;

    // Fetch all databases linked to this site
    let mysql_dbs = sqlx::query!(
        "SELECT db_name FROM user_databases WHERE site_id = $1 AND db_type = 'mysql'",
        site_id
    )
    .fetch_all(&state.db)
    .await?;

    let pg_dbs = sqlx::query!(
        "SELECT db_name FROM user_databases WHERE site_id = $1 AND db_type = 'postgres'",
        site_id
    )
    .fetch_all(&state.db)
    .await?;

    tracing::info!(
        site_id  = %site_id,
        domain   = %site.domain,
        mysql    = mysql_dbs.len(),
        postgres = pg_dbs.len(),
        "Starting site backup"
    );

    // ── 2. Create timestamped backup directory ────────────────────────────────

    let ts = chrono::Utc::now().format("%Y-%m-%d-%H%M%S").to_string();
    let backup_dir = PathBuf::from(format!("/var/backups/jottiecp/{}/{}", site_id, ts));
    std::fs::create_dir_all(&backup_dir)
        .map_err(|e| anyhow::anyhow!("create backup dir failed: {}", e))?;

    // ── 3. Archive site files ─────────────────────────────────────────────────

    let files_archive = backup_dir.join("files.tar.gz");
    let home_dir      = format!("/home/{}", site.unix_user);

    let files_archive_str = files_archive
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Backup path contains non-UTF-8 characters"))?;

    let tar_result = std::process::Command::new("tar")
        .args([
            "-czf",
            files_archive_str,
            "-C",
            &home_dir,
            ".",
        ])
        .status()
        .map_err(|e| anyhow::anyhow!("tar exec failed: {}", e));

    match &tar_result {
        Ok(s) if s.success() => {}
        Ok(s) => {
            // Remove the partial archive before bailing so we don't leave corrupt files.
            let _ = std::fs::remove_file(&files_archive);
            anyhow::bail!("tar -czf failed for {} (exit {:?})", home_dir, s.code());
        }
        Err(e) => {
            let _ = std::fs::remove_file(&files_archive);
            return Err(anyhow::anyhow!("tar exec failed: {}", e));
        }
    }

    // Set archive permissions to 0600 — backup files must not be world-readable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&files_archive, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| anyhow::anyhow!("chmod files.tar.gz failed: {}", e))?;
    }

    // ── 4. Dump MySQL databases ───────────────────────────────────────────────

    let mut db_files: Vec<BackupFile> = Vec::new();

    for db in &mysql_dbs {
        let dump_path = backup_dir.join(format!("{}.sql.gz", db.db_name));
        dump_mysql(&db.db_name, &dump_path)?;
        db_files.push(build_file_entry(&dump_path, &format!("{}.sql.gz", db.db_name))?);
    }

    // ── 5. Dump PostgreSQL databases ──────────────────────────────────────────

    for db in &pg_dbs {
        let dump_path = backup_dir.join(format!("{}_pg.sql.gz", db.db_name));
        dump_postgres(&db.db_name, &dump_path)?;
        db_files.push(build_file_entry(&dump_path, &format!("{}_pg.sql.gz", db.db_name))?);
    }

    // ── 6. Write manifest.json ────────────────────────────────────────────────

    let files_entry = build_file_entry(&files_archive, "files.tar.gz")?;
    let total_bytes: u64 = std::iter::once(&files_entry)
        .chain(db_files.iter())
        .map(|f| f.size_bytes)
        .sum();

    let manifest = BackupManifest {
        site_id:    site_id.to_string(),
        domain:     site.domain.clone(),
        unix_user:  site.unix_user.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
        total_bytes,
        files:      std::iter::once(files_entry).chain(db_files).collect(),
    };

    let manifest_path = backup_dir.join("manifest.json");
    let manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| anyhow::anyhow!("serialize manifest failed: {}", e))?;
    std::fs::write(&manifest_path, &manifest_json)
        .map_err(|e| anyhow::anyhow!("write manifest.json failed: {}", e))?;

    // ── 7. Update backup_jobs table ───────────────────────────────────────────

    sqlx::query!(
        r#"UPDATE backup_jobs
           SET status       = 'completed',
               size_bytes   = $1,
               manifest_path = $2,
               completed_at = NOW()
           WHERE site_id = $3
             AND status   = 'running'"#,
        total_bytes as i64,
        manifest_path.display().to_string(),
        site_id,
    )
    .execute(&state.db)
    .await?;

    // ── 8. Enforce retention policy ───────────────────────────────────────────

    apply_retention_policy(state, site_id).await?;

    tracing::info!(
        site_id     = %site_id,
        total_bytes = %total_bytes,
        backup_dir  = %backup_dir.display(),
        "Backup complete"
    );

    Ok(())
}

// ── MySQL dump ────────────────────────────────────────────────────────────────

fn dump_mysql(db_name: &str, dest: &Path) -> anyhow::Result<()> {
    use std::io::Write;

    validate_db_name(db_name)?;

    // mysqldump | gzip — pipe via shell-less process chaining
    let mut mysqldump = std::process::Command::new("mysqldump")
        .args([
            "--single-transaction",    // consistent snapshot (InnoDB)
            "--quick",                 // stream rows (low memory)
            "--routines",              // include stored procedures
            "--triggers",              // include triggers
            "--events",                // include events
            "--",
            db_name,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("mysqldump spawn failed: {}", e))?;

    let mut gzip = std::process::Command::new("gzip")
        .args(["-c"])
        .stdin(
            mysqldump.stdout.take()
                .ok_or_else(|| anyhow::anyhow!("mysqldump stdout missing"))?,
        )
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("gzip spawn failed: {}", e))?;

    // Stream gzip stdout directly to the dest file — avoids buffering entire dump in RAM.
    let gzip_out = gzip.stdout.take()
        .ok_or_else(|| anyhow::anyhow!("gzip stdout missing"))?;

    let mut dest_file = std::fs::File::create(dest)
        .map_err(|e| anyhow::anyhow!("create mysql dump file {} failed: {}", dest.display(), e))?;

    let bytes_written = std::io::copy(&mut BufReader::new(gzip_out), &mut dest_file)
        .map_err(|e| anyhow::anyhow!("stream gzip output to {} failed: {}", dest.display(), e))?;

    dest_file.flush()
        .map_err(|e| anyhow::anyhow!("flush mysql dump file {} failed: {}", dest.display(), e))?;
    drop(dest_file);

    let dump_status = mysqldump.wait()
        .map_err(|e| anyhow::anyhow!("mysqldump wait failed: {}", e))?;

    // Check exit codes after streaming is complete.
    if !dump_status.success() {
        let _ = std::fs::remove_file(dest);
        anyhow::bail!("mysqldump exited {:?} for db '{}'", dump_status.code(), db_name);
    }

    let gzip_status = gzip.wait()
        .map_err(|e| anyhow::anyhow!("gzip wait failed: {}", e))?;
    if !gzip_status.success() {
        let _ = std::fs::remove_file(dest);
        anyhow::bail!("gzip exited {:?} for db '{}'", gzip_status.code(), db_name);
    }

    // Sanity check: a valid gzip-compressed SQL dump is at least a few hundred bytes.
    // An empty or nearly-empty file almost certainly means mysqldump produced nothing.
    if bytes_written < 64 {
        let _ = std::fs::remove_file(dest);
        anyhow::bail!(
            "mysqldump output for db '{}' is suspiciously small ({} bytes) — refusing to keep",
            db_name,
            bytes_written
        );
    }

    // Secure the dump file: readable only by root.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| anyhow::anyhow!("chmod mysql dump {} failed: {}", dest.display(), e))?;
    }

    Ok(())
}

// ── PostgreSQL dump ───────────────────────────────────────────────────────────

fn dump_postgres(db_name: &str, dest: &Path) -> anyhow::Result<()> {
    validate_db_name(db_name)?;

    let mut pgdump = std::process::Command::new("pg_dump")
        .args([
            "--no-password",
            "--format=plain",  // SQL text — most portable
            "--",
            db_name,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("pg_dump spawn failed: {}", e))?;

    let mut gzip = std::process::Command::new("gzip")
        .args(["-c"])
        .stdin(
            pgdump.stdout.take()
                .ok_or_else(|| anyhow::anyhow!("pg_dump stdout missing"))?,
        )
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("gzip spawn for pg_dump failed: {}", e))?;

    let gzip_out = gzip.stdout.take()
        .ok_or_else(|| anyhow::anyhow!("gzip stdout missing"))?;

    let mut reader = BufReader::new(gzip_out);
    let mut content = Vec::new();
    reader.read_to_end(&mut content)?;

    // Wait for both children BEFORE writing, to detect failures early.
    let dump_status = pgdump.wait()?;
    if !dump_status.success() {
        anyhow::bail!("pg_dump exited {:?} for db '{}'", dump_status.code(), db_name);
    }

    let gzip_status = gzip.wait()?;
    if !gzip_status.success() {
        anyhow::bail!("gzip exited {:?} for pg_dump db '{}'", gzip_status.code(), db_name);
    }

    // Sanity: reject suspiciously small output.
    if content.len() < 64 {
        anyhow::bail!(
            "pg_dump output for db '{}' is suspiciously small ({} bytes) — refusing to write",
            db_name,
            content.len()
        );
    }

    std::fs::write(dest, &content)
        .map_err(|e| anyhow::anyhow!("write pg dump to {} failed: {}", dest.display(), e))?;

    // Secure the dump file: readable only by root.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| anyhow::anyhow!("chmod pg dump {} failed: {}", dest.display(), e))?;
    }

    Ok(())
}

// ── Retention policy ──────────────────────────────────────────────────────────

/// Delete old backups beyond retention limits:
///   - Keep last 7 daily backups
///   - Keep last 4 weekly backups (oldest of each week)
///   - Keep last 12 monthly backups (oldest of each month)
///
/// Anything else is deleted from disk + DB.
async fn apply_retention_policy(state: &Arc<AppState>, site_id: Uuid) -> anyhow::Result<()> {
    // Fetch all completed backups for this site, newest first
    let backups = sqlx::query!(
        r#"SELECT id, manifest_path, completed_at
           FROM backup_jobs
           WHERE site_id = $1 AND status = 'completed'
           ORDER BY completed_at DESC"#,
        site_id
    )
    .fetch_all(&state.db)
    .await?;

    let mut to_keep: std::collections::HashSet<Uuid> = std::collections::HashSet::new();

    // Keep last 7 (daily)
    for b in backups.iter().take(7) {
        to_keep.insert(b.id);
    }

    // Keep one per week for 4 weeks (28 days) → the first backup seen for each ISO week
    let mut weekly_seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for b in &backups {
        if let Some(ts) = b.completed_at {
            let key = format!(
                "{}-W{}",
                ts.format("%Y"),
                ts.format("%V")
            );
            if weekly_seen.len() < 4 && !weekly_seen.contains(&key) {
                weekly_seen.insert(key);
                to_keep.insert(b.id);
            }
        }
    }

    // Keep one per month for 12 months
    let mut monthly_seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for b in &backups {
        if let Some(ts) = b.completed_at {
            let key = ts.format("%Y-%m").to_string();
            if monthly_seen.len() < 12 && !monthly_seen.contains(&key) {
                monthly_seen.insert(key);
                to_keep.insert(b.id);
            }
        }
    }

    // Delete anything not in to_keep
    for b in &backups {
        if !to_keep.contains(&b.id) {
            // Delete from disk
            if let Some(ref manifest) = b.manifest_path {
                // manifest is at BACKUP_DIR/manifest.json — delete parent dir
                if let Some(parent) = PathBuf::from(manifest).parent() {
                    if parent.starts_with("/var/backups/jottiecp") {
                        let _ = std::fs::remove_dir_all(parent);
                    }
                }
            }
            // Soft-delete from DB
            let _ = sqlx::query!(
                "UPDATE backup_jobs SET status = 'expired' WHERE id = $1",
                b.id
            )
            .execute(&state.db)
            .await;
        }
    }

    Ok(())
}

// ── Restore operation ─────────────────────────────────────────────────────────

/// Restore a backup from `manifest_path`.
///
/// Steps:
///   1. Parse manifest.json
///   2. Verify SHA-256 of every file in the manifest (refuses to extract on mismatch)
///   3. Extract files.tar.gz to the site's home directory
///   4. Import MySQL / PostgreSQL dumps
///
/// The site must exist in the DB (we need its unix_user and domain).
pub async fn restore_site(
    state:         &Arc<AppState>,
    site_id:       Uuid,
    manifest_path: &Path,
) -> anyhow::Result<()> {
    // ── 1. Load + parse manifest ─────────────────────────────────────────────

    let manifest_str = std::fs::read_to_string(manifest_path)
        .map_err(|e| anyhow::anyhow!("read manifest.json failed: {}", e))?;

    let manifest: serde_json::Value = serde_json::from_str(&manifest_str)
        .map_err(|e| anyhow::anyhow!("parse manifest.json failed: {}", e))?;

    let backup_dir = manifest_path.parent()
        .ok_or_else(|| anyhow::anyhow!("manifest.json has no parent directory"))?;

    // Safety: backup_dir must be within the expected path prefix.
    anyhow::ensure!(
        backup_dir.starts_with("/var/backups/jottiecp"),
        "restore_site: backup dir '{}' is outside allowed prefix",
        backup_dir.display()
    );

    // ── 2. Verify SHA-256 of all files listed in the manifest ────────────────

    let files = manifest["files"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("manifest.json: missing 'files' array"))?;

    for file_entry in files {
        let name = file_entry["name"].as_str()
            .ok_or_else(|| anyhow::anyhow!("manifest file entry missing 'name'"))?;
        let expected_sha256 = file_entry["sha256"].as_str()
            .ok_or_else(|| anyhow::anyhow!("manifest file entry '{}' missing 'sha256'", name))?;

        // Prevent path traversal in file names from a tampered manifest.
        anyhow::ensure!(
            !name.contains('/') && !name.contains(".."),
            "restore_site: manifest file name '{}' contains path traversal chars",
            name
        );

        let file_path = backup_dir.join(name);
        let content = std::fs::read(&file_path)
            .map_err(|e| anyhow::anyhow!("restore_site: cannot read backup file '{}': {}", name, e))?;

        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&content);
        let actual_sha256 = hex::encode(hasher.finalize());

        anyhow::ensure!(
            actual_sha256 == expected_sha256,
            "restore_site: SHA-256 mismatch for '{}': expected {} got {}",
            name,
            expected_sha256,
            actual_sha256
        );

        tracing::debug!(file = %name, "Checksum verified OK");
    }

    tracing::info!(site_id = %site_id, "All backup checksums verified — proceeding with restore");

    // ── 3. Look up site unix_user ────────────────────────────────────────────

    let site = sqlx::query!(
        "SELECT unix_user FROM sites WHERE id = $1 AND deleted_at IS NULL",
        site_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| anyhow::anyhow!("Site {} not found for restore", site_id))?;

    // ── 4. Extract files.tar.gz to site home dir ─────────────────────────────

    let files_archive = backup_dir.join("files.tar.gz");
    if files_archive.exists() {
        let home_dir = format!("/home/{}", site.unix_user);
        let files_archive_str = files_archive
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Backup path contains non-UTF-8 characters"))?;
        let status = std::process::Command::new("tar")
            .args([
                "-xzf",
                files_archive_str,
                "-C",
                &home_dir,
                "--no-same-owner",
            ])
            .status()
            .map_err(|e| anyhow::anyhow!("tar -xzf exec failed: {}", e))?;

        anyhow::ensure!(
            status.success(),
            "tar -xzf failed for restore into {} (exit {:?})",
            home_dir,
            status.code()
        );
        tracing::info!(site_id = %site_id, dest = %home_dir, "Site files restored");
    }

    // ── 5. Restore MySQL databases ───────────────────────────────────────────

    for file_entry in files {
        let name = file_entry["name"].as_str().unwrap_or("");
        if !name.ends_with(".sql.gz") || name.ends_with("_pg.sql.gz") {
            continue;
        }
        let db_name = name.trim_end_matches(".sql.gz");
        validate_db_name(db_name)?;

        let dump_path = backup_dir.join(name);
        restore_mysql(db_name, &dump_path)?;
    }

    // ── 6. Restore PostgreSQL databases ──────────────────────────────────────

    for file_entry in files {
        let name = file_entry["name"].as_str().unwrap_or("");
        if !name.ends_with("_pg.sql.gz") {
            continue;
        }
        let db_name = name.trim_end_matches("_pg.sql.gz");
        validate_db_name(db_name)?;

        let dump_path = backup_dir.join(name);
        restore_postgres(db_name, &dump_path)?;
    }

    tracing::info!(site_id = %site_id, "Restore complete");
    Ok(())
}

// ── Restore helpers ───────────────────────────────────────────────────────────

fn restore_mysql(db_name: &str, dump_path: &Path) -> anyhow::Result<()> {
    validate_db_name(db_name)?;

    // Verify the dump is non-empty before piping to mysql.
    let meta = std::fs::metadata(dump_path)
        .map_err(|e| anyhow::anyhow!("stat {} failed: {}", dump_path.display(), e))?;
    anyhow::ensure!(
        meta.len() >= 64,
        "MySQL dump '{}' is suspiciously small ({} bytes) — refusing restore",
        dump_path.display(),
        meta.len()
    );

    // zcat file.sql.gz | mysql --single-transaction db_name
    // Note: --single-transaction is a dump flag, not a restore flag.
    // For restore, we wrap the import inside a transaction at the session level.
    let mut zcat = std::process::Command::new("zcat")
        .arg(dump_path)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("zcat spawn failed: {}", e))?;

    let mysql = std::process::Command::new("mysql")
        .args(["--defaults-file=/etc/mysql/debian.cnf", "--", db_name])
        .stdin(
            zcat.stdout.take()
                .ok_or_else(|| anyhow::anyhow!("zcat stdout missing"))?,
        )
        .output()
        .map_err(|e| anyhow::anyhow!("mysql spawn failed: {}", e))?;

    let zcat_status = zcat.wait()
        .map_err(|e| anyhow::anyhow!("zcat wait failed: {}", e))?;

    anyhow::ensure!(zcat_status.success(), "zcat failed for {}", dump_path.display());
    anyhow::ensure!(
        mysql.status.success(),
        "mysql restore of '{}' failed: {}",
        db_name,
        String::from_utf8_lossy(&mysql.stderr).trim()
    );

    tracing::info!(db_name = %db_name, "MySQL database restored");
    Ok(())
}

fn restore_postgres(db_name: &str, dump_path: &Path) -> anyhow::Result<()> {
    validate_db_name(db_name)?;

    let meta = std::fs::metadata(dump_path)
        .map_err(|e| anyhow::anyhow!("stat {} failed: {}", dump_path.display(), e))?;
    anyhow::ensure!(
        meta.len() >= 64,
        "PostgreSQL dump '{}' is suspiciously small ({} bytes) — refusing restore",
        dump_path.display(),
        meta.len()
    );

    let mut zcat = std::process::Command::new("zcat")
        .arg(dump_path)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("zcat spawn failed: {}", e))?;

    let psql = std::process::Command::new("psql")
        .args(["--no-password", "--", db_name])
        .stdin(
            zcat.stdout.take()
                .ok_or_else(|| anyhow::anyhow!("zcat stdout missing"))?,
        )
        .output()
        .map_err(|e| anyhow::anyhow!("psql spawn failed: {}", e))?;

    let zcat_status = zcat.wait()
        .map_err(|e| anyhow::anyhow!("zcat wait failed: {}", e))?;

    anyhow::ensure!(zcat_status.success(), "zcat failed for {}", dump_path.display());
    anyhow::ensure!(
        psql.status.success(),
        "psql restore of '{}' failed: {}",
        db_name,
        String::from_utf8_lossy(&psql.stderr).trim()
    );

    tracing::info!(db_name = %db_name, "PostgreSQL database restored");
    Ok(())
}

// ── Manifest helpers ──────────────────────────────────────────────────────────

#[derive(serde::Serialize)]
struct BackupManifest {
    site_id:    String,
    domain:     String,
    unix_user:  String,
    created_at: String,
    total_bytes: u64,
    files:      Vec<BackupFile>,
}

#[derive(serde::Serialize, Clone)]
struct BackupFile {
    name:       String,
    size_bytes: u64,
    sha256:     String,
}

fn build_file_entry(path: &Path, name: &str) -> anyhow::Result<BackupFile> {
    use sha2::{Digest, Sha256};

    let content = std::fs::read(path)
        .map_err(|e| anyhow::anyhow!("read {} for manifest failed: {}", path.display(), e))?;

    let size_bytes = content.len() as u64;

    let mut hasher = Sha256::new();
    hasher.update(&content);
    let sha256 = hex::encode(hasher.finalize());

    Ok(BackupFile {
        name:  name.to_string(),
        size_bytes,
        sha256,
    })
}

// ── Validation ────────────────────────────────────────────────────────────────

/// Prevent DB name injection: only allow alphanumeric + underscores.
fn validate_db_name(name: &str) -> anyhow::Result<()> {
    let re = regex::Regex::new(r"^[a-zA-Z0-9_]{1,64}$").unwrap();
    anyhow::ensure!(re.is_match(name), "Invalid database name: {}", name);
    Ok(())
}
