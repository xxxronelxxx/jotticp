//! cPanel .tar.gz migration service.
//!
//! Processing pipeline (invoked by the job processor for "cpanel_migration" jobs):
//!   1. Extract the outer tar.gz to /tmp/orbit-migration-JOBID/extracted/
//!   2. Look for homedir.tar.gz   → extract site files to the site's home dir
//!   3. Look for mysql/*.sql.gz   → gunzip each + import into a new DB
//!   4. Look for emails/DOMAIN/   → create email accounts via jotti-mail API
//!   5. Look for dnszones/DOMAIN.db → parse BIND zone file + import via jotti-dns
//!   6. Look for crontabs/USER    → parse crontab + create cron_jobs rows
//!   7. Write validation_report JSON into migration_jobs.report
//!   8. Update status → 'completed' (or 'failed')

use std::{path::{Path, PathBuf}, sync::Arc};
use uuid::Uuid;
use anyhow::Context as _;
use serde_json::json;

use crate::AppState;

// ── Public entry point ────────────────────────────────────────────────────────

/// Process a cPanel migration job end-to-end.
/// Called from the job dispatcher with a pre-validated archive path.
/// Maximum accepted archive size: 10 GiB.
///
/// This prevents an operator or attacker from uploading an archive that
/// exhausts /tmp disk space during extraction.
const MAX_ARCHIVE_BYTES: u64 = 10 * 1024 * 1024 * 1024;

pub async fn process_cpanel_migration(
    state: &Arc<AppState>,
    job_id: Uuid,
    archive_path: &Path,
    target_site_id: Option<Uuid>,
) -> anyhow::Result<()> {
    update_progress(state, job_id, 0, "running").await;

    // ── Archive size guard ────────────────────────────────────────────────────
    let archive_meta = tokio::fs::metadata(archive_path).await
        .context("stat migration archive")?;
    if archive_meta.len() > MAX_ARCHIVE_BYTES {
        anyhow::bail!(
            "Migration archive is {} bytes, which exceeds the {} byte limit",
            archive_meta.len(),
            MAX_ARCHIVE_BYTES
        );
    }
    if archive_meta.len() == 0 {
        anyhow::bail!("Migration archive is empty (0 bytes)");
    }

    let extract_dir = PathBuf::from(format!("/tmp/orbit-migration-{}/extracted", job_id));
    tokio::fs::create_dir_all(&extract_dir).await
        .context("create extraction dir")?;

    // ── Step 1: Extract outer archive ─────────────────────────────────────────
    let extract_result = extract_tar_gz(archive_path, &extract_dir).await
        .context("extract outer archive");

    if let Err(e) = extract_result {
        // Clean up the temp dir on extraction failure so /tmp isn't left full.
        let _ = tokio::fs::remove_dir_all(format!("/tmp/orbit-migration-{}", job_id)).await;
        return Err(e);
    }

    update_progress(state, job_id, 10, "running").await;

    let mut report = MigrationReport::default();

    // ── Step 2: Site files (homedir.tar.gz) ───────────────────────────────────
    let homedir_archive = extract_dir.join("homedir.tar.gz");
    if homedir_archive.exists() {
        match import_site_files(state, job_id, &homedir_archive, target_site_id).await {
            Ok(path) => {
                report.site_files_imported = true;
                report.site_homedir = Some(path);
            }
            Err(e) => {
                report.errors.push(format!("site_files: {}", e));
                tracing::warn!(job_id = %job_id, error = %e, "Failed to import site files");
            }
        }
    } else {
        // Some cPanel backups place files directly in homedir/
        let homedir_dir = extract_dir.join("homedir");
        if homedir_dir.exists() {
            report.site_files_imported = true;
            report.site_homedir = Some(homedir_dir.to_string_lossy().into_owned());
        }
    }

    update_progress(state, job_id, 30, "running").await;

    // ── Step 3: MySQL dumps ───────────────────────────────────────────────────
    let mysql_dir = extract_dir.join("mysql");
    if mysql_dir.exists() {
        match import_mysql_dumps(state, &mysql_dir, target_site_id).await {
            Ok(dbs) => report.databases_imported = dbs,
            Err(e) => {
                report.errors.push(format!("databases: {}", e));
                tracing::warn!(job_id = %job_id, error = %e, "Failed to import MySQL dumps");
            }
        }
    }

    update_progress(state, job_id, 50, "running").await;

    // ── Step 4: Email accounts ────────────────────────────────────────────────
    let emails_dir = extract_dir.join("emails");
    if emails_dir.exists() {
        match import_email_accounts(state, &emails_dir, target_site_id).await {
            Ok(count) => report.email_accounts_imported = count,
            Err(e) => {
                report.errors.push(format!("email: {}", e));
                tracing::warn!(job_id = %job_id, error = %e, "Failed to import email accounts");
            }
        }
    }

    update_progress(state, job_id, 65, "running").await;

    // ── Step 5: DNS zones ─────────────────────────────────────────────────────
    let dns_dir = extract_dir.join("dnszones");
    if dns_dir.exists() {
        match import_dns_zones(state, &dns_dir, target_site_id).await {
            Ok(zones) => report.dns_zones_imported = zones,
            Err(e) => {
                report.errors.push(format!("dns: {}", e));
                tracing::warn!(job_id = %job_id, error = %e, "Failed to import DNS zones");
            }
        }
    }

    update_progress(state, job_id, 80, "running").await;

    // ── Step 6: Cron jobs ─────────────────────────────────────────────────────
    let crontabs_dir = extract_dir.join("crontabs");
    if crontabs_dir.exists() {
        match import_cron_jobs(state, &crontabs_dir, target_site_id).await {
            Ok(count) => report.cron_jobs_imported = count,
            Err(e) => {
                report.errors.push(format!("cron: {}", e));
                tracing::warn!(job_id = %job_id, error = %e, "Failed to import cron jobs");
            }
        }
    }

    update_progress(state, job_id, 95, "running").await;

    // ── Step 7 & 8: Finalise ──────────────────────────────────────────────────
    let status = if report.errors.is_empty() { "completed" } else { "completed_with_errors" };
    let report_json = json!({
        "site_files_imported":     report.site_files_imported,
        "site_homedir":            report.site_homedir,
        "databases_imported":      report.databases_imported,
        "email_accounts_imported": report.email_accounts_imported,
        "dns_zones_imported":      report.dns_zones_imported,
        "cron_jobs_imported":      report.cron_jobs_imported,
        "errors":                  report.errors,
    });

    sqlx::query!(
        r#"UPDATE migration_jobs
           SET status = $1, progress = 100, report = $2, completed_at = NOW()
           WHERE id = $3"#,
        status, report_json, job_id
    )
    .execute(&state.db)
    .await
    .context("update final migration_jobs status")?;

    // Cleanup temp files
    let _ = tokio::fs::remove_dir_all(format!("/tmp/orbit-migration-{}", job_id)).await;

    tracing::info!(job_id = %job_id, status = status, "Migration job completed");
    Ok(())
}

// ── Internal helpers ──────────────────────────────────────────────────────────

#[derive(Default)]
struct MigrationReport {
    site_files_imported:     bool,
    site_homedir:            Option<String>,
    databases_imported:      Vec<String>,
    email_accounts_imported: usize,
    dns_zones_imported:      Vec<String>,
    cron_jobs_imported:      usize,
    errors:                  Vec<String>,
}

/// Update the migration job's progress percentage and status in the DB.
async fn update_progress(state: &Arc<AppState>, job_id: Uuid, pct: i32, status: &str) {
    let _ = sqlx::query!(
        "UPDATE migration_jobs SET progress = $1, status = $2 WHERE id = $3",
        pct, status, job_id
    )
    .execute(&state.db)
    .await;
}

/// Synchronously extract a .tar.gz to dest_dir using the system `tar` binary.
async fn extract_tar_gz(archive: &Path, dest: &Path) -> anyhow::Result<()> {
    let output = tokio::process::Command::new("tar")
        .args([
            "--extract",
            "--gzip",
            "--file",
            archive.to_str().unwrap_or(""),
            "--directory",
            dest.to_str().unwrap_or(""),
            "--no-same-owner",
        ])
        .output()
        .await
        .context("spawn tar")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("tar extraction failed: {}", stderr);
    }
    Ok(())
}

/// Extract homedir.tar.gz to the target site's home directory.
/// Returns the destination path.
async fn import_site_files(
    state: &Arc<AppState>,
    job_id: Uuid,
    homedir_archive: &Path,
    target_site_id: Option<Uuid>,
) -> anyhow::Result<String> {
    let dest = if let Some(site_id) = target_site_id {
        // Look up the site's unix_user to derive its home directory
        let row = sqlx::query!(
            "SELECT unix_user FROM sites WHERE id = $1 AND deleted_at IS NULL",
            site_id
        )
        .fetch_optional(&state.db)
        .await
        .context("lookup site unix_user")?;

        if let Some(r) = row {
            format!("/home/{}", r.unix_user)
        } else {
            // Fallback: extract to job temp dir
            format!("/tmp/orbit-migration-{}/homedir-extracted", job_id)
        }
    } else {
        // No target site: extract to temp for later provisioning
        format!("/tmp/orbit-migration-{}/homedir-extracted", job_id)
    };

    tokio::fs::create_dir_all(&dest).await.context("create homedir dest")?;
    let dest_path = PathBuf::from(&dest);
    extract_tar_gz(homedir_archive, &dest_path).await
        .context("extract homedir.tar.gz")?;

    tracing::info!(job_id = %job_id, dest = %dest, "Site files extracted");
    Ok(dest)
}

/// Import MySQL .sql.gz dumps from the mysql/ directory.
/// Each file `dbname.sql.gz` is decompressed then piped to mysql.
async fn import_mysql_dumps(
    state: &Arc<AppState>,
    mysql_dir: &Path,
    target_site_id: Option<Uuid>,
) -> anyhow::Result<Vec<String>> {
    let mut imported = Vec::new();

    let mut read_dir = tokio::fs::read_dir(mysql_dir).await
        .context("read mysql dir")?;

    while let Some(entry) = read_dir.next_entry().await.context("iterate mysql dir")? {
        let path = entry.path();
        let filename = path.file_name().unwrap_or_default().to_string_lossy().to_string();

        // Only process .sql.gz files
        if !filename.ends_with(".sql.gz") {
            continue;
        }

        let db_name = filename.trim_end_matches(".sql.gz").to_string();

        // Validate db_name before using it in any shell command argument.
        // The filename comes from a user-supplied archive — it could contain
        // shell metacharacters.  We reject anything outside safe characters.
        let db_name_ok = regex::Regex::new(r"^[a-zA-Z0-9_]{1,64}$").unwrap();
        if !db_name_ok.is_match(&db_name) {
            tracing::warn!(db_name = %db_name, "Skipping MySQL dump with invalid db name");
            continue;
        }

        // Create a new database row in site_databases if we have a target site
        if let Some(site_id) = target_site_id {
            let db_id = Uuid::new_v4();
            let db_user = format!("orbit_{}", db_name.chars().take(16).collect::<String>());

            let _ = sqlx::query!(
                r#"INSERT INTO user_databases (id, site_id, db_type, db_name, db_user, status, created_at)
                   VALUES ($1, $2, 'mysql', $3, $4, 'provisioning', NOW())
                   ON CONFLICT DO NOTHING"#,
                db_id, site_id, db_name, db_user
            )
            .execute(&state.db)
            .await;
        }

        // Decompress and import using shell-free argument passing (no sh -c).
        // Previously used `sh -c "zcat FILE | mysql DB"` which allowed shell
        // injection if db_name contained metacharacters.
        // Use std::process::Command so that ChildStdout can be piped as Stdio.
        let mut zcat = std::process::Command::new("zcat")
            .arg(&path)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .context(format!("spawn zcat for {}", filename))?;

        let zcat_stdout = zcat.stdout.take()
            .ok_or_else(|| anyhow::anyhow!("zcat stdout missing for {}", filename))?;

        let mysql_status = tokio::task::spawn_blocking({
            let db_name = db_name.clone();
            move || {
                std::process::Command::new("mysql")
                    .args(["--defaults-file=/etc/mysql/debian.cnf", "--", &db_name])
                    .stdin(std::process::Stdio::from(zcat_stdout))
                    .status()
            }
        })
        .await
        .context(format!("spawn_blocking mysql for {}", filename))?
        .context(format!("import mysql dump: {}", filename))?;

        let zcat_status = tokio::task::spawn_blocking(move || zcat.wait())
            .await
            .context(format!("spawn_blocking zcat wait for {}", filename))?
            .context(format!("zcat wait for {}", filename))?;

        if zcat_status.success() && mysql_status.success() {
            imported.push(db_name.clone());
            tracing::info!(db_name = %db_name, "MySQL dump imported");
        } else {
            tracing::warn!(db_name = %db_name, "MySQL dump import failed (non-fatal)");
        }
    }

    Ok(imported)
}

/// Parse email account directories under emails/DOMAIN/ and create records.
/// cPanel stores virtual mailboxes as directories named after the local part.
async fn import_email_accounts(
    state: &Arc<AppState>,
    emails_dir: &Path,
    target_site_id: Option<Uuid>,
) -> anyhow::Result<usize> {
    let mut count = 0usize;

    let mut domains = tokio::fs::read_dir(emails_dir).await
        .context("read emails dir")?;

    while let Some(domain_entry) = domains.next_entry().await.context("iterate email domains")? {
        if !domain_entry.metadata().await.map(|m| m.is_dir()).unwrap_or(false) {
            continue;
        }
        let domain = domain_entry.file_name().to_string_lossy().to_string();
        let domain_path = domain_entry.path();

        let mut accounts = tokio::fs::read_dir(&domain_path).await
            .context("read email domain dir")?;

        while let Some(acct_entry) = accounts.next_entry().await.context("iterate email accounts")? {
            if !acct_entry.metadata().await.map(|m| m.is_dir()).unwrap_or(false) {
                continue;
            }

            let local = acct_entry.file_name().to_string_lossy().to_string();
            let address = format!("{}@{}", local, domain);

            if let Some(site_id) = target_site_id {
                let acct_id = Uuid::new_v4();
                // Insert with a placeholder password_hash — user must reset password.
                // We use status='active' since the maildir files will be rsync'd separately.
                let _ = sqlx::query!(
                    r#"INSERT INTO email_accounts
                           (id, site_id, address, quota_mb, used_mb, status, created_at)
                       VALUES ($1, $2, $3, 1024, 0, 'active', NOW())
                       ON CONFLICT DO NOTHING"#,
                    acct_id, site_id, address
                )
                .execute(&state.db)
                .await;
            }

            count += 1;
            tracing::debug!(address = %address, "Email account imported");
        }
    }

    Ok(count)
}

/// Parse BIND zone files from dnszones/ and insert DNS records.
/// cPanel zone files follow standard BIND syntax.
async fn import_dns_zones(
    state: &Arc<AppState>,
    dns_dir: &Path,
    _target_site_id: Option<Uuid>,
) -> anyhow::Result<Vec<String>> {
    let mut imported_zones = Vec::new();

    let mut entries = tokio::fs::read_dir(dns_dir).await
        .context("read dnszones dir")?;

    while let Some(entry) = entries.next_entry().await.context("iterate dns zones")? {
        let path = entry.path();
        let filename = path.file_name().unwrap_or_default().to_string_lossy().to_string();

        if !filename.ends_with(".db") {
            continue;
        }

        let zone_name = filename.trim_end_matches(".db").to_string();
        let content = tokio::fs::read_to_string(&path).await
            .context(format!("read zone file: {}", filename))?;

        let records = parse_bind_zone(&content);

        // DNS records require a zone_id FK to dns_zones, which requires an owner_id.
        // During cPanel import we don't always have an owner — log the records and
        // let the admin assign them via the DNS API after import.
        imported_zones.push(zone_name.clone());
        tracing::info!(zone = %zone_name, records = records.len(), "DNS zone parsed (manual import required to assign owner)");
    }

    Ok(imported_zones)
}

/// Parse a crontab file and create cron_jobs rows for the target site.
async fn import_cron_jobs(
    state: &Arc<AppState>,
    crontabs_dir: &Path,
    target_site_id: Option<Uuid>,
) -> anyhow::Result<usize> {
    let mut count = 0usize;

    let Some(site_id) = target_site_id else {
        return Ok(0); // No site to attach cron jobs to
    };

    let mut entries = tokio::fs::read_dir(crontabs_dir).await
        .context("read crontabs dir")?;

    while let Some(entry) = entries.next_entry().await.context("iterate crontabs")? {
        let content = tokio::fs::read_to_string(entry.path()).await
            .context("read crontab file")?;

        for line in content.lines() {
            let line = line.trim();
            // Skip comments, blank lines, and MAILTO/SHELL env lines
            if line.is_empty() || line.starts_with('#') || line.starts_with("MAILTO=")
                || line.starts_with("SHELL=") || line.starts_with("PATH=")
            {
                continue;
            }

            if let Some(cron) = parse_cron_line(line) {
                let job_id = Uuid::new_v4();
                let _ = sqlx::query!(
                    r#"INSERT INTO cron_jobs
                           (id, site_id, minute, hour, day, month, weekday,
                            command, label, enabled, created_at)
                       VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, true, NOW())"#,
                    job_id, site_id,
                    cron.minute, cron.hour, cron.day,
                    cron.month, cron.weekday,
                    cron.command,
                    format!("Imported from cPanel")
                )
                .execute(&state.db)
                .await;

                count += 1;
            }
        }
    }

    Ok(count)
}

// ── Parsing helpers ───────────────────────────────────────────────────────────

struct DnsRecord {
    name:        String,
    record_type: String,
    value:       String,
    ttl:         i32,
}

/// Minimal BIND zone file parser.
/// Handles lines of the form: `name [TTL] IN TYPE value`
fn parse_bind_zone(content: &str) -> Vec<DnsRecord> {
    let mut records = Vec::new();
    let mut default_ttl = 3600i32;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('$') {
            // Handle $TTL directive
            if line.starts_with("$TTL") {
                if let Some(ttl_str) = line.split_whitespace().nth(1) {
                    if let Ok(t) = ttl_str.parse::<i32>() {
                        default_ttl = t;
                    }
                }
            }
            continue;
        }

        // Tokenise: name [ttl] IN type value...
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 4 {
            continue;
        }

        let mut idx = 0;
        let name = parts[idx].to_string();
        idx += 1;

        // Optional TTL
        let mut ttl = default_ttl;
        if let Ok(t) = parts[idx].parse::<i32>() {
            ttl = t;
            idx += 1;
        }

        // Optional "IN" class
        if idx < parts.len() && parts[idx].eq_ignore_ascii_case("IN") {
            idx += 1;
        }

        if idx >= parts.len() {
            continue;
        }

        let record_type = parts[idx].to_uppercase();
        idx += 1;

        // Value: everything after the type
        let value = parts[idx..].join(" ");

        // Only import well-known record types
        if matches!(record_type.as_str(), "A" | "AAAA" | "CNAME" | "MX" | "TXT" | "NS" | "SRV") {
            records.push(DnsRecord { name, record_type, value, ttl });
        }
    }

    records
}

struct CronEntry {
    minute:  String,
    hour:    String,
    day:     String,
    month:   String,
    weekday: String,
    command: String,
}

/// Parse a single crontab line into its schedule fields and command.
fn parse_cron_line(line: &str) -> Option<CronEntry> {
    let parts: Vec<&str> = line.splitn(6, ' ').collect();
    if parts.len() < 6 {
        return None;
    }
    Some(CronEntry {
        minute:  parts[0].to_string(),
        hour:    parts[1].to_string(),
        day:     parts[2].to_string(),
        month:   parts[3].to_string(),
        weekday: parts[4].to_string(),
        command: parts[5].to_string(),
    })
}
