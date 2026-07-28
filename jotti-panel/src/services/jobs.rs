use crate::{
    AppState,
    services::{
        provisioning::{self, ProvisioningJob},
        ssl,
        backup,
        monitor,
        migration,
    },
};
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

// ── Public API ────────────────────────────────────────────────────────────────

/// Start the background job processor in a dedicated Tokio task.
///
/// The processor uses a tight loop:
///   - Job available → process immediately, then check again
///   - Queue empty   → sleep 1s, then check again
///   - Error         → log, sleep 5s (back-pressure)
pub fn start_job_processor(state: Arc<AppState>) {
    tokio::spawn(async move {
        tracing::info!("Job processor started (Valkey list: orbit:jobs)");
        loop {
            match process_next_job(&state).await {
                Ok(true)  => {} // processed — try next immediately
                Ok(false) => tokio::time::sleep(Duration::from_secs(1)).await,
                Err(e)    => {
                    tracing::error!(error = %e, "Job processing error");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    });
}

// ── Core dispatch loop ────────────────────────────────────────────────────────

/// Pop one job from the Valkey list and dispatch it.
///
/// Returns:
///   Ok(true)  — job was processed (successfully or with error logged)
///   Ok(false) — queue was empty
///   Err(_)    — unexpected infrastructure error (Valkey down, JSON parse fail)
///
/// # Known limitation — at-most-once delivery
///
/// `RPOP` is destructive: the job is removed from the queue before processing
/// begins.  If the worker process crashes after RPOP but before the dispatch
/// function returns, the job is permanently lost.
///
/// To achieve at-least-once delivery, replace `RPOP` with a two-step pattern:
///   1. `BRPOPLPUSH orbit:jobs orbit:jobs:inflight 0` — moves job to an
///      "in-flight" list atomically.
///   2. After successful processing, `LREM orbit:jobs:inflight 1 <job>`.
///   3. A reaper task periodically re-queues jobs stuck in orbit:jobs:inflight
///      for longer than a configurable timeout.
///
/// Until that is implemented, callers should make every dispatch function
/// idempotent (e.g. check DB status before acting, as done in provision_site).
async fn process_next_job(state: &Arc<AppState>) -> anyhow::Result<bool> {
    use redis::AsyncCommands;

    let job_str: Option<String> = {
        let mut conn = state.valkey.clone();
        conn.rpop("orbit:jobs", None).await?
    };

    let Some(job_str) = job_str else {
        return Ok(false);
    };

    let job: serde_json::Value = serde_json::from_str(&job_str)
        .map_err(|e| anyhow::anyhow!("Invalid job JSON: {}: {}", e, &job_str[..job_str.len().min(256)]))?;

    let job_type = job["type"].as_str().unwrap_or("").to_string();

    tracing::info!(job_type = %job_type, "Processing job");

    match job_type.as_str() {
        "provision_site"     => dispatch_provision_site(state, &job).await,
        "deprovision_site"   => dispatch_deprovision_site(state, &job).await,
        "issue_ssl"          => dispatch_issue_ssl(state, &job).await,
        "install_ssl"        => dispatch_install_ssl(state, &job).await,
        "suspend_site"       => dispatch_suspend_site(state, &job).await,
        "unsuspend_site"     => dispatch_unsuspend_site(state, &job).await,
        "change_php_version" => dispatch_change_php_version(state, &job).await,
        "backup_site"        => dispatch_backup_site(state, &job).await,
        "run_backup"         => dispatch_backup_site(state, &job).await,
        "restore_backup"     => dispatch_restore_backup(state, &job).await,
        "delete_backup_files"=> dispatch_delete_backup_files(state, &job).await,
        // Database lifecycle
        "provision_database"   => dispatch_provision_database(state, &job).await,
        "deprovision_database" => dispatch_deprovision_database(state, &job).await,
        "change_db_password"   => dispatch_change_db_password(state, &job).await,
        // PHP / cache
        "flush_opcache"        => dispatch_flush_opcache(state, &job).await,
        "reload_php_pool"      => dispatch_reload_php_pool(state, &job).await,
        "update_cache_headers" => dispatch_update_cache_headers(state, &job).await,
        // Email
        "generate_dkim_key"    => dispatch_generate_dkim_key(state, &job).await,
        "set_spam_threshold"   => dispatch_set_spam_threshold(state, &job).await,
        "update_email_quota"   => dispatch_update_email_quota(state, &job).await,
        "sync_autoresponder"   => dispatch_sync_autoresponder(state, &job).await,
        "sync_email_forwarder" => dispatch_sync_email_forwarder().await,
        // Cron
        "run_cron_now"         => dispatch_run_cron_now(state, &job).await,
        "sync_cron"            => dispatch_sync_cron(state, &job).await,
        // Reseller bulk
        "suspend_user_sites"   => dispatch_set_user_sites_suspended(state, &job, true).await,
        "unsuspend_user_sites" => dispatch_set_user_sites_suspended(state, &job, false).await,
        // SSL
        "revoke_ssl"           => dispatch_revoke_ssl(state, &job).await,
        "activate_custom_ssl"  => dispatch_activate_custom_ssl(state, &job).await,
        // DNS via PowerDNS (pdnsutil)
        "provision_dns_zone"   => dispatch_dns_zone(state, &job, "provision").await,
        "deprovision_dns_zone" => dispatch_dns_zone(state, &job, "deprovision").await,
        "sync_dns_zone"        => dispatch_dns_zone(state, &job, "sync").await,
        "sync_dns_record"      => dispatch_sync_dns_record(state, &job).await,
        "migrate_plesk"        => dispatch_migrate_plesk(state, &job).await,
        "collect_stats"      => dispatch_collect_stats(state).await,
        "renew_ssl"          => dispatch_renew_ssl(state).await,
        // Runtime provisioning jobs (Steps 4.9–4.12)
        "setup_python_site"  => dispatch_setup_runtime(state, &job, "python").await,
        "setup_nodejs_site"  => dispatch_setup_runtime(state, &job, "nodejs").await,
        "setup_ruby_site"    => dispatch_setup_runtime(state, &job, "ruby").await,
        "setup_dotnet_site"  => dispatch_setup_runtime(state, &job, "dotnet").await,
        // Database provisioning jobs (Steps 4.4–4.6)
        "provision_valkey"    => dispatch_provision_valkey(state, &job).await,
        "provision_surrealdb" => dispatch_provision_surrealdb(state, &job).await,
        // start_valkey is enqueued by dispatch_provision_valkey above; jotti-agent
        // starts the Valkey process.  We forward it via the agent job mechanism.
        "start_valkey"        => dispatch_start_valkey(state, &job).await,
        // cPanel migration import (Step 4.15)
        "cpanel_migration"    => dispatch_cpanel_migration(state, &job).await,
        // 1-click app installer (apps.rs)
        "install_app"         => dispatch_install_app(state, &job).await,
        "update_app"          => dispatch_update_app(state, &job).await,
        // Email provisioning
        "provision_email_account"   => dispatch_provision_email(state, &job).await,
        "change_email_password"     => dispatch_change_email_password(state, &job).await,
        "deprovision_email_account" => dispatch_deprovision_email(state, &job).await,
        other => {
            tracing::warn!(job_type = other, raw = %job_str, "Unknown job type — discarding");
        }
    }

    Ok(true)
}

// ── Dispatchers ───────────────────────────────────────────────────────────────

async fn dispatch_provision_site(state: &Arc<AppState>, job: &serde_json::Value) {
    let site_id    = parse_uuid(job, "site_id");
    let domain     = job["domain"].as_str().unwrap_or("").to_string();
    let php_version = job["php_version"].as_str().unwrap_or("8.3").to_string();
    let web_server  = job["web_server"].as_str().unwrap_or("nginx").to_string();
    let unix_user   = job["unix_user"].as_str().unwrap_or("").to_string();
    let server_id   = job["server_id"].as_str().and_then(|s| Uuid::parse_str(s).ok());

    if site_id.is_nil() || domain.is_empty() || unix_user.is_empty() {
        tracing::error!(job = %job, "provision_site: missing required fields");
        return;
    }

    let pjob = ProvisioningJob {
        site_id,
        domain,
        php_version,
        web_server,
        unix_user,
        server_id,
    };

    if let Err(e) = provisioning::provision_site(state, pjob).await {
        tracing::error!(site_id = %site_id, error = %e, "provision_site failed");
    }
}

async fn dispatch_deprovision_site(state: &Arc<AppState>, job: &serde_json::Value) {
    let site_id   = parse_uuid(job, "site_id");
    let domain    = job["domain"].as_str().unwrap_or("").to_string();
    let unix_user = job["unix_user"].as_str().unwrap_or("").to_string();

    if site_id.is_nil() || domain.is_empty() || unix_user.is_empty() {
        tracing::error!(job = %job, "deprovision_site: missing required fields");
        return;
    }

    if let Some(addr) = site_agent_addr(state, site_id).await {
        if let Err(e) = provisioning::agent_delete_site(&addr, &domain, true).await {
            tracing::error!(site_id = %site_id, error = %e, "deprovision_site (agent) failed");
        }
    } else if let Err(e) = provisioning::deprovision_site(state, site_id, &domain, &unix_user).await {
        tracing::error!(site_id = %site_id, error = %e, "deprovision_site failed");
    }
}

async fn dispatch_issue_ssl(state: &Arc<AppState>, job: &serde_json::Value) {
    let site_id  = parse_uuid(job, "site_id");
    let domain   = job["domain"].as_str().unwrap_or("").to_string();
    let challenge = job["challenge"].as_str().unwrap_or("http-01").to_string();

    if site_id.is_nil() || domain.is_empty() {
        tracing::error!(job = %job, "issue_ssl: missing required fields");
        return;
    }

    if let Err(e) = ssl::issue_certificate(state, site_id, &domain, &challenge).await {
        tracing::error!(site_id = %site_id, domain = %domain, error = %e, "issue_ssl failed");

        // Upsert ssl_certs status to 'error' so the UI shows the failure.
        // Using INSERT ... ON CONFLICT so this also creates the row if cert
        // issuance failed before upsert_ssl_cert was reached (first-time failure).
        let _ = sqlx::query!(
            r#"INSERT INTO ssl_certs (id, site_id, domain, status, expires_at, cert_path, key_path, created_at)
               VALUES ($1, $2, $3, 'error', NOW() + INTERVAL '1 day', '', '', NOW())
               ON CONFLICT (site_id)
               DO UPDATE SET status = 'error', updated_at = NOW()"#,
            uuid::Uuid::new_v4(),
            site_id,
            domain,
        )
        .execute(&state.db)
        .await;
    }
}

async fn dispatch_install_ssl(state: &Arc<AppState>, job: &serde_json::Value) {
    let domain    = job["domain"].as_str().unwrap_or("").to_string();
    let cert_path = job["cert_path"].as_str().unwrap_or("").to_string();
    let key_path  = job["key_path"].as_str().unwrap_or("").to_string();

    if domain.is_empty() || cert_path.is_empty() || key_path.is_empty() {
        tracing::error!(job = %job, "install_ssl: missing required fields");
        return;
    }

    if let Err(e) = ssl::install_ssl_vhost(state, &domain, &cert_path, &key_path).await {
        tracing::error!(domain = %domain, error = %e, "install_ssl failed");
    }
}

async fn dispatch_suspend_site(state: &Arc<AppState>, job: &serde_json::Value) {
    let site_id    = parse_uuid(job, "site_id");
    let domain_raw = job["domain"].as_str().unwrap_or("").to_string();

    // Look up domain from DB if not included in the job payload
    let domain = if !domain_raw.is_empty() {
        domain_raw
    } else {
        match sqlx::query_scalar!("SELECT domain FROM sites WHERE id = $1", site_id)
            .fetch_optional(&state.db)
            .await
        {
            Ok(Some(d)) => d,
            _ => {
                tracing::error!(site_id = %site_id, "suspend_site: cannot find domain");
                return;
            }
        }
    };

    if let Some(addr) = site_agent_addr(state, site_id).await {
        if let Err(e) = provisioning::agent_suspend_site(&addr, &domain).await {
            tracing::error!(site_id = %site_id, error = %e, "suspend_site (agent) failed");
        }
    } else if let Err(e) = provisioning::suspend_site_files(site_id, &domain).await {
        tracing::error!(site_id = %site_id, error = %e, "suspend_site failed");
    }
}

async fn dispatch_unsuspend_site(state: &Arc<AppState>, job: &serde_json::Value) {
    let site_id = parse_uuid(job, "site_id");

    if site_id.is_nil() {
        tracing::error!(job = %job, "unsuspend_site: missing site_id");
        return;
    }

    if let Some(addr) = site_agent_addr(state, site_id).await {
        let domain = sqlx::query_scalar!("SELECT domain FROM sites WHERE id=$1", site_id)
            .fetch_optional(&state.db).await.ok().flatten().unwrap_or_default();
        if let Err(e) = provisioning::agent_unsuspend_site(&addr, &domain).await {
            tracing::error!(site_id = %site_id, error = %e, "unsuspend_site (agent) failed");
        }
    } else if let Err(e) = provisioning::unsuspend_site_files(state, site_id).await {
        tracing::error!(site_id = %site_id, error = %e, "unsuspend_site failed");
    }
}

async fn dispatch_change_php_version(state: &Arc<AppState>, job: &serde_json::Value) {
    let site_id = parse_uuid(job, "site_id");
    let version = job["version"].as_str().unwrap_or("").to_string();

    if site_id.is_nil() || version.is_empty() {
        tracing::error!(job = %job, "change_php_version: missing required fields");
        return;
    }

    // Remote server (enrolled via agent) → swap the FPM pool over gRPC mTLS instead of locally.
    if let Some(addr) = site_agent_addr(state, site_id).await {
        if let Some((domain, unix_user, _)) = site_meta(state, site_id).await {
            match provisioning::agent_set_php_version(&addr, &domain, &version, &unix_user).await {
                Ok(()) => {
                    let _ = sqlx::query!("UPDATE sites SET php_version=$2 WHERE id=$1", site_id, version).execute(&state.db).await;
                    tracing::info!(site_id = %site_id, version = %version, addr = %addr, "php version changed via agent");
                }
                Err(e) => tracing::error!(site_id = %site_id, error = %e, "agent php version change failed"),
            }
        }
        return;
    }

    if let Err(e) = provisioning::change_php_version(state, site_id, &version).await {
        tracing::error!(site_id = %site_id, version = %version, error = %e, "change_php_version failed");
    }
}

async fn dispatch_backup_site(state: &Arc<AppState>, job: &serde_json::Value) {
    let site_id = parse_uuid(job, "site_id");

    if site_id.is_nil() {
        tracing::error!(job = %job, "backup_site: missing site_id");
        return;
    }

    // The API (POST /backups/{site_id}) already inserted the backup_jobs row as
    // 'pending' with id = backup_id. Flip it to 'running' so backup_site (which
    // updates WHERE site_id AND status='running') targets this exact row.
    let backup_id = parse_uuid(job, "backup_id");
    let _ = sqlx::query!(
        "UPDATE backup_jobs SET status = 'running' WHERE id = $1",
        backup_id
    )
    .execute(&state.db)
    .await;

    if let Err(e) = backup::backup_site(state, site_id).await {
        tracing::error!(site_id = %site_id, error = %e, "backup_site failed");

        let _ = sqlx::query!(
            "UPDATE backup_jobs SET status = 'failed', completed_at = NOW() WHERE site_id = $1 AND status = 'running'",
            site_id
        )
        .execute(&state.db)
        .await;
    }
}

async fn dispatch_restore_backup(state: &Arc<AppState>, job: &serde_json::Value) {
    let site_id = parse_uuid(job, "site_id");
    let target = job["target"].as_str().unwrap_or("same");
    let manifest_path = job["manifest_path"].as_str().unwrap_or("");
    if site_id.is_nil() || manifest_path.is_empty() {
        tracing::error!(job = %job, "restore_backup: missing site_id or manifest_path");
        return;
    }
    if target != "same" {
        tracing::warn!(job = %job, "restore_backup: target '{}' not yet supported (only 'same')", target);
        return;
    }
    match backup::restore_site(state, site_id, std::path::Path::new(manifest_path)).await {
        Ok(())  => tracing::info!(site_id = %site_id, "restore_backup complete"),
        Err(e)  => tracing::error!(site_id = %site_id, error = %e, "restore_backup failed"),
    }
}

async fn dispatch_delete_backup_files(_state: &Arc<AppState>, job: &serde_json::Value) {
    let manifest_path = job["manifest_path"].as_str().unwrap_or("");
    if manifest_path.is_empty() { return; }
    let p = std::path::Path::new(manifest_path);
    match p.parent() {
        Some(dir) if dir.starts_with("/var/backups/jottiecp") => {
            if let Err(e) = std::fs::remove_dir_all(dir) {
                tracing::warn!(dir = %dir.display(), error = %e, "delete_backup_files: cleanup failed");
            } else {
                tracing::info!(dir = %dir.display(), "delete_backup_files: removed backup dir");
            }
        }
        _ => tracing::warn!(path = manifest_path, "delete_backup_files: refusing to delete outside backup root"),
    }
}

// ── Helpers for the wired-up dispatchers ──────────────────────────────────────

/// Run a system command (fire-and-check); returns Ok on spawn. Local to this module.
fn run_cmd(cmd: &str, args: &[&str]) -> std::io::Result<std::process::Output> {
    std::process::Command::new(cmd).args(args).output()
}

/// Only allow [A-Za-z0-9_] identifiers — DB names/users come from validated input
/// but we re-check here since they are interpolated into SQL.
fn safe_ident(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// If the site lives on a remote (enrolled) server, return that agent's address
/// so the operation is routed over gRPC mTLS instead of run locally.
async fn site_agent_addr(state: &Arc<AppState>, site_id: Uuid) -> Option<String> {
    sqlx::query!(
        "SELECT s.agent_address FROM sites si JOIN servers s ON s.id = si.server_id \
         WHERE si.id = $1 AND si.deleted_at IS NULL AND s.deleted_at IS NULL",
        site_id
    ).fetch_optional(&state.db).await.ok().flatten().and_then(|r| r.agent_address)
}

/// If a DNS zone belongs to a remote (enrolled) server, return that server's agent address.
async fn zone_agent_addr(state: &Arc<AppState>, zone_id: Uuid) -> Option<String> {
    if zone_id.is_nil() { return None; }
    sqlx::query!(
        "SELECT s.agent_address FROM dns_zones z JOIN servers s ON s.id = z.server_id \
         WHERE z.id = $1 AND z.deleted_at IS NULL AND s.deleted_at IS NULL",
        zone_id
    ).fetch_optional(&state.db).await.ok().flatten().and_then(|r| r.agent_address)
}

async fn site_meta(state: &Arc<AppState>, site_id: Uuid) -> Option<(String, String, String)> {
    sqlx::query!(
        "SELECT domain, unix_user, php_version FROM sites WHERE id = $1 AND deleted_at IS NULL",
        site_id
    )
    .fetch_optional(&state.db).await.ok().flatten()
    .map(|r| (r.domain, r.unix_user, r.php_version))
}

async fn run_mysql(sql: &str) -> bool {
    tokio::process::Command::new("mysql").arg("-e").arg(sql).output().await
        .map(|o| o.status.success()).unwrap_or(false)
}
async fn run_psql(sql: &str) -> bool {
    tokio::process::Command::new("sudo").args(["-u", "postgres", "psql", "-tAc", sql]).output().await
        .map(|o| o.status.success()).unwrap_or(false)
}

// ── Database lifecycle ────────────────────────────────────────────────────────

async fn dispatch_provision_database(state: &Arc<AppState>, job: &serde_json::Value) {
    let db_id   = parse_uuid(job, "db_id");
    let db_type = job["db_type"].as_str().unwrap_or("");
    let db_name = job["db_name"].as_str().unwrap_or("");
    let db_user = job["db_user"].as_str().unwrap_or("");
    if db_id.is_nil() || !safe_ident(db_name) || !safe_ident(db_user) {
        tracing::error!(job = %job, "provision_database: missing/invalid fields");
        let _ = sqlx::query!("UPDATE user_databases SET status='error' WHERE id=$1", db_id).execute(&state.db).await;
        return;
    }
    let pwd = generate_random_hex(16);
    // Remote server (enrolled via agent) → provision over gRPC mTLS instead of locally.
    if let Some(addr) = site_agent_addr(state, parse_uuid(job, "site_id")).await {
        match provisioning::agent_create_database(&addr, db_type, db_name, db_user, &pwd).await {
            Ok(()) => {
                let _ = sqlx::query!("UPDATE user_databases SET status='active', db_password=$2 WHERE id=$1", db_id, pwd)
                    .execute(&state.db).await;
                tracing::info!(db = %db_name, addr = %addr, "database provisioned via agent");
            }
            Err(e) => {
                let _ = sqlx::query!("UPDATE user_databases SET status='error' WHERE id=$1", db_id).execute(&state.db).await;
                tracing::error!(db = %db_name, error = %e, "agent database provisioning failed");
            }
        }
        return;
    }
    let ok = match db_type {
        "mysql" | "mariadb" => run_mysql(&format!(
            "CREATE DATABASE IF NOT EXISTS `{db_name}` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci; \
             CREATE USER IF NOT EXISTS '{db_user}'@'%' IDENTIFIED BY '{pwd}'; \
             GRANT ALL PRIVILEGES ON `{db_name}`.* TO '{db_user}'@'%'; FLUSH PRIVILEGES;")).await,
        "postgres" | "postgresql" => {
            run_psql(&format!("CREATE ROLE \"{db_user}\" LOGIN PASSWORD '{pwd}';")).await
            && run_psql(&format!("CREATE DATABASE \"{db_name}\" OWNER \"{db_user}\";")).await
        }
        other => { tracing::warn!(db_type = other, "provision_database: unsupported db_type"); false }
    };
    if ok {
        let _ = sqlx::query!("UPDATE user_databases SET status='active', db_password=$2 WHERE id=$1", db_id, pwd)
            .execute(&state.db).await;
        tracing::info!(db = %db_name, "database provisioned");
    } else {
        let _ = sqlx::query!("UPDATE user_databases SET status='error' WHERE id=$1", db_id).execute(&state.db).await;
        tracing::error!(db = %db_name, "database provisioning failed");
    }
}

async fn dispatch_deprovision_database(state: &Arc<AppState>, job: &serde_json::Value) {
    let db_type = job["db_type"].as_str().unwrap_or("");
    let db_name = job["db_name"].as_str().unwrap_or("");
    let db_user = job["db_user"].as_str().unwrap_or("");
    if !safe_ident(db_name) { tracing::error!(job=%job, "deprovision_database: bad db_name"); return; }
    // Remote server (enrolled via agent) → drop over gRPC mTLS instead of locally.
    if let Some(addr) = site_agent_addr(state, parse_uuid(job, "site_id")).await {
        match provisioning::agent_delete_database(&addr, db_type, db_name, db_user).await {
            Ok(()) => tracing::info!(db = %db_name, addr = %addr, "database deprovisioned via agent"),
            Err(e) => tracing::error!(db = %db_name, error = %e, "agent database deprovision failed"),
        }
        return;
    }
    match db_type {
        "mysql" | "mariadb" => {
            let mut sql = format!("DROP DATABASE IF EXISTS `{db_name}`;");
            if safe_ident(db_user) { sql.push_str(&format!(" DROP USER IF EXISTS '{db_user}'@'%';")); }
            sql.push_str(" FLUSH PRIVILEGES;");
            run_mysql(&sql).await;
        }
        "postgres" | "postgresql" => {
            run_psql(&format!("DROP DATABASE IF EXISTS \"{db_name}\";")).await;
            if safe_ident(db_user) { run_psql(&format!("DROP ROLE IF EXISTS \"{db_user}\";")).await; }
        }
        _ => {}
    }
    tracing::info!(db = %db_name, "database deprovisioned");
}

async fn dispatch_change_db_password(state: &Arc<AppState>, job: &serde_json::Value) {
    let db_id   = parse_uuid(job, "db_id");
    let db_type = job["db_type"].as_str().unwrap_or("");
    let db_user = job["db_user"].as_str().unwrap_or("");
    let pwd     = job["new_password"].as_str().unwrap_or("");
    if !safe_ident(db_user) || pwd.is_empty() || pwd.contains('\'') {
        tracing::error!(job=%job, "change_db_password: bad user/password"); return;
    }
    let ok = match db_type {
        "mysql" | "mariadb" => run_mysql(&format!(
            "ALTER USER '{db_user}'@'%' IDENTIFIED BY '{pwd}'; FLUSH PRIVILEGES;")).await,
        "postgres" | "postgresql" => run_psql(&format!("ALTER ROLE \"{db_user}\" PASSWORD '{pwd}';")).await,
        _ => false,
    };
    if ok && !db_id.is_nil() {
        let _ = sqlx::query!("UPDATE user_databases SET db_password=$2 WHERE id=$1", db_id, pwd).execute(&state.db).await;
        tracing::info!(user = %db_user, "db password changed");
    } else if !ok { tracing::error!(user = %db_user, "db password change failed"); }
}

// ── PHP / cache ───────────────────────────────────────────────────────────────

async fn dispatch_flush_opcache(state: &Arc<AppState>, job: &serde_json::Value) {
    let site_id = parse_uuid(job, "site_id");
    if let Some((_d, user, ver)) = site_meta(state, site_id).await {
        if let Err(e) = crate::services::cache::flush_opcache(&ver, &user).await {
            tracing::warn!(error = %e, "flush_opcache failed");
        }
    }
}

async fn dispatch_reload_php_pool(state: &Arc<AppState>, job: &serde_json::Value) {
    let site_id = parse_uuid(job, "site_id");
    let Some((_d, user, ver)) = site_meta(state, site_id).await else {
        tracing::error!(job = %job, "reload_php_pool: site not found"); return;
    };
    let pool_path = format!("/etc/php/{}/fpm/pool.d/{}.conf", ver, user);
    // Re-apply the site's custom PHP settings + enabled extensions into the pool conf,
    // inside an idempotent managed block, so changes actually take effect at runtime
    // (the DB rows alone have no effect on php-fpm — they must be written to the pool).
    match std::fs::read_to_string(&pool_path) {
        Ok(conf) => {
            const BEGIN: &str = "; >>> jottiecp-custom (managed; do not edit)";
            const END:   &str = "; <<< jottiecp-custom";
            let mut base = if let (Some(s), Some(e)) = (conf.find(BEGIN), conf.find(END)) {
                let mut t = conf.clone();
                t.replace_range(s..e + END.len(), "");
                t.trim_end().to_string()
            } else {
                conf.trim_end().to_string()
            };
            let settings = sqlx::query!(
                "SELECT setting_key, setting_value FROM site_php_settings WHERE site_id=$1", site_id
            ).fetch_all(&state.db).await.unwrap_or_default();
            let exts = sqlx::query!(
                "SELECT extension_name FROM site_php_extensions WHERE site_id=$1 AND enabled=true", site_id
            ).fetch_all(&state.db).await.unwrap_or_default();
            // sanitize to prevent pool-conf injection
            let ok_key = |k: &str| !k.is_empty() && k.len() < 64
                && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_');
            let ok_val = |v: &str| v.len() < 256
                && !v.contains(['\n', '\r', '[', ']']);
            let ok_ext = |e: &str| !e.is_empty() && e.len() < 64
                && e.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            let mut block = String::from("\n");
            block.push_str(BEGIN); block.push('\n');
            for s in &settings {
                if ok_key(&s.setting_key) && ok_val(&s.setting_value) {
                    block.push_str(&format!("php_admin_value[{}] = {}\n", s.setting_key, s.setting_value));
                }
            }
            for e in &exts {
                if ok_ext(&e.extension_name) {
                    block.push_str(&format!("php_admin_value[extension] = {}.so\n", e.extension_name));
                }
            }
            block.push_str(END); block.push('\n');
            base.push('\n'); base.push_str(&block);
            if let Err(e) = std::fs::write(&pool_path, &base) {
                tracing::error!(error = %e, pool = %pool_path, "reload_php_pool: write pool conf failed");
            }
        }
        Err(e) => tracing::warn!(error = %e, pool = %pool_path, "reload_php_pool: pool conf not found"),
    }
    // SIGUSR2 graceful reload re-reads the pool config (settings) and reloads extensions.
    let _ = run_cmd("systemctl", &["reload", &format!("php{}-fpm", ver)]);
    tracing::info!(php = %ver, user = %user, "php-fpm pool reloaded with custom settings/extensions");
}

async fn dispatch_update_cache_headers(state: &Arc<AppState>, job: &serde_json::Value) {
    let site_id = parse_uuid(job, "site_id");
    let preset  = job["preset"].as_str().unwrap_or("balanced");
    if let Some((domain, _u, _v)) = site_meta(state, site_id).await {
        if crate::services::cache::validate_cache_preset(preset).is_ok() {
            if let Err(e) = crate::services::cache::apply_cache_preset(&domain, preset).await {
                tracing::warn!(error = %e, "update_cache_headers failed");
            }
        }
    }
}

// ── Email ─────────────────────────────────────────────────────────────────────

async fn dispatch_generate_dkim_key(_state: &Arc<AppState>, job: &serde_json::Value) {
    let domain = job["domain"].as_str().unwrap_or("");
    if domain.is_empty() { tracing::error!("generate_dkim_key: no domain"); return; }
    match crate::services::email::enable_dkim(domain).await {
        Ok(_) => tracing::info!(domain, "DKIM key generated"),
        Err(e) => tracing::warn!(domain, error = %e, "generate_dkim_key failed"),
    }
}

async fn dispatch_set_spam_threshold(_state: &Arc<AppState>, job: &serde_json::Value) {
    // The threshold is persisted in the DB by the API and consulted by the mail
    // filter at delivery time; nothing further to apply on this host.
    tracing::info!(job = %job, "set_spam_threshold: recorded (applied at delivery)");
}

async fn dispatch_update_email_quota(_state: &Arc<AppState>, job: &serde_json::Value) {
    // Dovecot reads the quota from the DB on login; ask it to recalc if we have the address.
    if let Some(addr) = job["address"].as_str() {
        let _ = tokio::process::Command::new("doveadm").args(["quota", "recalc", "-u", addr]).output().await;
    }
    tracing::info!(job = %job, "update_email_quota applied");
}

async fn dispatch_sync_email_forwarder() {
    // Forwarders are served live from the DB via Postfix virtual_alias_maps (pgsql);
    // reloading Postfix is enough to pick up changes immediately.
    let _ = run_cmd("systemctl", &["reload", "postfix"]);
    tracing::info!("email forwarders synced (postfix reloaded; map is DB-driven)");
}

async fn dispatch_sync_autoresponder(state: &Arc<AppState>, job: &serde_json::Value) {
    let ar_id  = parse_uuid(job, "autoresponder_id");
    let action = job["action"].as_str().unwrap_or("create");
    let row = sqlx::query!(
        "SELECT a.local_part, d.domain, ar.subject, ar.body \
         FROM email_autoresponders ar \
         JOIN email_accounts a ON a.id = ar.account_id \
         JOIN email_domains  d ON d.id = a.domain_id \
         WHERE ar.id = $1", ar_id
    ).fetch_optional(&state.db).await.ok().flatten();
    let Some(r) = row else { tracing::error!(job = %job, "sync_autoresponder: not found"); return; };
    // sanitise path components
    if r.domain.contains('/') || r.local_part.contains('/') || r.domain.contains("..") {
        tracing::error!("sync_autoresponder: bad path"); return;
    }
    let dir   = format!("/var/mail/vhosts/{}/{}", r.domain, r.local_part);
    let sieve = format!("{}/.dovecot.sieve", dir);
    if action == "delete" {
        let _ = std::fs::remove_file(&sieve);
        let _ = std::fs::remove_file(format!("{}/.dovecot.svbin", dir));
        tracing::info!(domain = %r.domain, "autoresponder removed");
        return;
    }
    let addr = format!("{}@{}", r.local_part, r.domain);
    let subj_raw = if r.subject.is_empty() { "Out of office".to_string() } else { r.subject };
    let subj = subj_raw.replace('"', "'").replace('\n', " ");
    let body = r.body.replace('"', "'");
    let script = format!(
        "require [\"vacation\"];\nvacation\n  :days 1\n  :subject \"{subj}\"\n  :addresses [\"{addr}\"]\n  \"{body}\";\n"
    );
    if std::fs::create_dir_all(&dir).is_ok() && std::fs::write(&sieve, script.as_bytes()).is_ok() {
        let _ = run_cmd("chown", &["-R", "vmail:vmail", &dir]);
        tracing::info!(addr = %addr, "autoresponder sieve written");
    } else {
        tracing::warn!(addr = %addr, "autoresponder: failed to write sieve");
    }
}

// ── Cron ──────────────────────────────────────────────────────────────────────

async fn dispatch_run_cron_now(_state: &Arc<AppState>, job: &serde_json::Value) {
    let user = job["unix_user"].as_str().unwrap_or("");
    let cmd  = job["command"].as_str().unwrap_or("");
    if user.is_empty() || cmd.is_empty()
        || !user.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        tracing::error!(job = %job, "run_cron_now: bad user/command"); return;
    }
    let out = tokio::process::Command::new("runuser")
        .args(["-u", user, "--", "/bin/sh", "-c", cmd]).output().await;
    match out {
        Ok(o) => tracing::info!(user, success = o.status.success(), "run_cron_now executed"),
        Err(e) => tracing::warn!(user, error = %e, "run_cron_now failed to launch"),
    }
}

async fn dispatch_sync_cron(state: &Arc<AppState>, job: &serde_json::Value) {
    // Regenerate the user's crontab from the cron_jobs table for this unix_user.
    let user = job["unix_user"].as_str().unwrap_or("");
    if user.is_empty() || !user.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        tracing::error!(job=%job, "sync_cron: bad user"); return;
    }
    let rows = sqlx::query!(
        "SELECT schedule, command FROM cron_jobs cj JOIN sites s ON s.id=cj.site_id \
         WHERE s.unix_user=$1 AND cj.deleted_at IS NULL AND cj.enabled=true", user
    ).fetch_all(&state.db).await.unwrap_or_default();
    let mut tab = String::from("# Managed by JottiCP\n");
    for r in &rows { tab.push_str(&format!("{} {}\n", r.schedule, r.command)); }
    // crontab -u <user> - (read from stdin)
    use std::io::Write;
    if let Ok(mut child) = std::process::Command::new("crontab").args(["-u", user, "-"])
        .stdin(std::process::Stdio::piped()).spawn() {
        if let Some(si) = child.stdin.as_mut() { let _ = si.write_all(tab.as_bytes()); }
        let _ = child.wait();
        tracing::info!(user, jobs = rows.len(), "crontab synced");
    }
}

// ── Reseller bulk suspend ─────────────────────────────────────────────────────

async fn dispatch_set_user_sites_suspended(state: &Arc<AppState>, job: &serde_json::Value, suspend: bool) {
    let user_id = parse_uuid(job, "user_id");
    if user_id.is_nil() { tracing::error!(job=%job, "suspend_user_sites: no user_id"); return; }
    let sites = sqlx::query!(
        "SELECT id, domain FROM sites WHERE owner_id=$1 AND deleted_at IS NULL", user_id
    ).fetch_all(&state.db).await.unwrap_or_default();
    for s in &sites {
        if suspend {
            let _ = crate::services::provisioning::suspend_site_files(s.id, &s.domain).await;
            let _ = sqlx::query!("UPDATE sites SET status='suspended' WHERE id=$1", s.id).execute(&state.db).await;
        } else {
            let _ = crate::services::provisioning::unsuspend_site_files(state, s.id).await;
            let _ = sqlx::query!("UPDATE sites SET status='active' WHERE id=$1", s.id).execute(&state.db).await;
        }
    }
    tracing::info!(user_id = %user_id, count = sites.len(), suspend, "bulk site suspend toggled");
}

// ── SSL ───────────────────────────────────────────────────────────────────────

async fn dispatch_revoke_ssl(state: &Arc<AppState>, job: &serde_json::Value) {
    let domain = job["domain"].as_str().unwrap_or("");
    if domain.is_empty() || domain.contains('/') || domain.contains("..") {
        tracing::error!(job=%job, "revoke_ssl: bad domain"); return;
    }
    let dir = format!("/etc/jottiecp/ssl/{}", domain);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = run_cmd("systemctl", &["reload", "nginx"]);
    let cert_id = parse_uuid(job, "cert_id");
    if !cert_id.is_nil() {
        let _ = sqlx::query!("UPDATE ssl_certs SET status='revoked' WHERE id=$1", cert_id).execute(&state.db).await;
    }
    tracing::info!(domain, "ssl revoked");
}

async fn dispatch_activate_custom_ssl(state: &Arc<AppState>, job: &serde_json::Value) {
    let domain    = job["domain"].as_str().unwrap_or("");
    let cert_path = job["cert_path"].as_str().unwrap_or("");
    let key_path  = job["key_path"].as_str().unwrap_or("");
    if domain.is_empty() || domain.contains('/') || domain.contains("..")
        || cert_path.is_empty() || key_path.is_empty() {
        tracing::error!(job=%job, "activate_custom_ssl: bad params"); return;
    }
    let dir = format!("/etc/jottiecp/ssl/{}", domain);
    let _ = std::fs::create_dir_all(&dir);
    let ok = std::fs::copy(cert_path, format!("{}/cert.pem", dir)).is_ok()
          && std::fs::copy(key_path,  format!("{}/key.pem",  dir)).is_ok();
    if ok {
        let _ = run_cmd("systemctl", &["reload", "nginx"]);
        let cert_id = parse_uuid(job, "cert_id");
        if !cert_id.is_nil() {
            let _ = sqlx::query!("UPDATE ssl_certs SET status='active' WHERE id=$1", cert_id).execute(&state.db).await;
        }
        tracing::info!(domain, "custom ssl activated");
    } else {
        tracing::warn!(domain, "activate_custom_ssl: copy failed");
    }
}

// ── Not-yet-supported on this host (handled, not silently discarded) ──────────

// ── DNS via PowerDNS (pdnsutil) ───────────────────────────────────────────────

/// pdnsutil wants the record name RELATIVE to the zone (it appends the zone),
/// but the DB stores absolute FQDNs — strip the zone suffix. Apex → "@".
fn dns_relative(name: &str, fqdn: &str) -> String {
    let n = name.trim_end_matches('.');
    let z = fqdn.trim_end_matches('.');
    if n.is_empty() || n == "@" || n == z { return "@".to_string(); }
    n.strip_suffix(&format!(".{}", z)).map(|s| s.to_string()).unwrap_or_else(|| n.to_string())
}

fn apply_pdns_record(fqdn: &str, name: &str, rtype: &str, content: &str, ttl: i32, priority: Option<i32>) {
    let ttl_s = (if ttl > 0 { ttl } else { 3600 }).to_string();
    let rel = dns_relative(name, fqdn);
    let content_full = if matches!(rtype, "MX" | "SRV") {
        format!("{} {}", priority.unwrap_or(10), content)
    } else { content.to_string() };
    let _ = std::process::Command::new("pdnsutil")
        .args(["replace-rrset", fqdn, &rel, rtype, &ttl_s, &content_full]).output();
}

async fn dispatch_dns_zone(state: &Arc<AppState>, job: &serde_json::Value, action: &str) {
    let domain  = job["domain"].as_str().unwrap_or("");
    let zone_id = parse_uuid(job, "zone_id");
    if domain.is_empty() || crate::services::provisioning::validate_domain_pub(domain).is_err() {
        tracing::error!(job = %job, "dns_zone: bad domain"); return;
    }
    let fqdn = format!("{}.", domain.trim_end_matches('.'));
    // Remote server (enrolled via agent) → manage the zone AND its records on the remote
    // PowerDNS over gRPC mTLS (CreateDnsZone + per-record UpsertDnsRecord).
    if let Some(addr) = zone_agent_addr(state, zone_id).await {
        match action {
            "provision" | "sync" => match provisioning::agent_create_dns_zone(&addr, domain).await {
                Ok(()) => {
                    if !zone_id.is_nil() {
                        let recs = sqlx::query!(
                            r#"SELECT name, COALESCE(record_type, type) AS "rtype!", content, ttl, priority
                               FROM dns_records WHERE zone_id=$1 AND deleted_at IS NULL"#, zone_id
                        ).fetch_all(&state.db).await.unwrap_or_default();
                        for r in &recs {
                            if let Err(e) = provisioning::agent_upsert_dns_record(
                                &addr, domain, &r.name, &r.rtype, &r.content, r.ttl, r.priority.unwrap_or(0)).await {
                                tracing::warn!(domain, record = %r.name, error = %e, "agent record push failed");
                            }
                        }
                    }
                    let _ = sqlx::query!("UPDATE dns_zones SET status='active' WHERE id=$1", zone_id).execute(&state.db).await;
                    tracing::info!(domain, addr = %addr, "dns zone + records synced via agent");
                }
                Err(e) => tracing::error!(domain, error = %e, "agent dns zone create failed"),
            },
            "deprovision" => match provisioning::agent_delete_dns_zone(&addr, domain).await {
                Ok(()) => tracing::info!(domain, addr = %addr, "dns zone deleted via agent"),
                Err(e) => tracing::error!(domain, error = %e, "agent dns zone delete failed"),
            },
            _ => {}
        }
        return;
    }
    let pdnsutil = |args: &[&str]| std::process::Command::new("pdnsutil").args(args).output()
        .map(|o| o.status.success()).unwrap_or(false);
    match action {
        "provision" | "sync" => {
            if !pdnsutil(&["list-zone", &fqdn]) {
                pdnsutil(&["create-zone", &fqdn, &format!("ns1.{}.", domain.trim_end_matches('.'))]);
            }
            if !zone_id.is_nil() {
                let recs = sqlx::query!(
                    r#"SELECT name, COALESCE(record_type, type) AS "rtype!", content, ttl, priority
                       FROM dns_records WHERE zone_id=$1 AND deleted_at IS NULL"#, zone_id
                ).fetch_all(&state.db).await.unwrap_or_default();
                for r in &recs { apply_pdns_record(&fqdn, &r.name, &r.rtype, &r.content, r.ttl, r.priority); }
                let _ = std::process::Command::new("pdnsutil").args(["rectify-zone", &fqdn]).output();
                let _ = sqlx::query!("UPDATE dns_zones SET status='active' WHERE id=$1", zone_id).execute(&state.db).await;
            }
            tracing::info!(domain, action, "dns zone applied via PowerDNS");
        }
        "deprovision" => {
            pdnsutil(&["delete-zone", &fqdn]);
            tracing::info!(domain, "dns zone deleted from PowerDNS");
        }
        _ => {}
    }
}

async fn dispatch_sync_dns_record(state: &Arc<AppState>, job: &serde_json::Value) {
    let record_id = parse_uuid(job, "record_id");
    let zone_id   = parse_uuid(job, "zone_id");
    let action    = job["action"].as_str().unwrap_or("create");
    if zone_id.is_nil() { tracing::error!(job = %job, "sync_dns_record: no zone_id"); return; }
    let Some(zone) = sqlx::query!(r#"SELECT COALESCE(zone, domain) AS "zname!" FROM dns_zones WHERE id=$1"#, zone_id)
        .fetch_optional(&state.db).await.ok().flatten() else {
        tracing::error!("sync_dns_record: zone not found"); return; };
    let zdomain = zone.zname;
    if zdomain.is_empty() { tracing::error!("sync_dns_record: zone has no domain"); return; }
    let fqdn = format!("{}.", zdomain.trim_end_matches('.'));
    // Remote server (enrolled via agent) → manage the record on its PowerDNS over gRPC mTLS.
    if let Some(addr) = zone_agent_addr(state, zone_id).await {
        if action == "delete" {
            if let Some(r) = sqlx::query!(
                r#"SELECT name, COALESCE(record_type, type) AS "rtype!" FROM dns_records WHERE id=$1"#, record_id
            ).fetch_optional(&state.db).await.ok().flatten() {
                match provisioning::agent_delete_dns_record(&addr, &zdomain, &r.name, &r.rtype).await {
                    Ok(()) => tracing::info!(zone = %zdomain, addr = %addr, "dns record deleted via agent"),
                    Err(e) => tracing::error!(zone = %zdomain, error = %e, "agent dns record delete failed"),
                }
            }
        } else if let Some(r) = sqlx::query!(
            r#"SELECT name, COALESCE(record_type, type) AS "rtype!", content, ttl, priority
               FROM dns_records WHERE id=$1 AND deleted_at IS NULL"#, record_id
        ).fetch_optional(&state.db).await.ok().flatten() {
            match provisioning::agent_upsert_dns_record(&addr, &zdomain, &r.name, &r.rtype, &r.content, r.ttl, r.priority.unwrap_or(0)).await {
                Ok(()) => tracing::info!(zone = %zdomain, addr = %addr, "dns record upserted via agent"),
                Err(e) => tracing::error!(zone = %zdomain, error = %e, "agent dns record upsert failed"),
            }
        }
        return;
    }
    if action == "delete" {
        if let Some(r) = sqlx::query!(
            r#"SELECT name, COALESCE(record_type, type) AS "rtype!" FROM dns_records WHERE id=$1"#, record_id
        ).fetch_optional(&state.db).await.ok().flatten() {
            let rel = dns_relative(&r.name, &fqdn);
            let _ = std::process::Command::new("pdnsutil").args(["delete-rrset", &fqdn, &rel, &r.rtype]).output();
        }
    } else if let Some(r) = sqlx::query!(
        r#"SELECT name, COALESCE(record_type, type) AS "rtype!", content, ttl, priority
           FROM dns_records WHERE id=$1 AND deleted_at IS NULL"#, record_id
    ).fetch_optional(&state.db).await.ok().flatten() {
        apply_pdns_record(&fqdn, &r.name, &r.rtype, &r.content, r.ttl, r.priority);
    }
    let _ = std::process::Command::new("pdnsutil").args(["rectify-zone", &fqdn]).output();
    tracing::info!(zone = %zdomain, action, "dns record synced");
}

// ── Plesk migration (basic importer) ─────────────────────────────────────────

async fn dispatch_migrate_plesk(_state: &Arc<AppState>, job: &serde_json::Value) {
    let archive = job["archive_path"].as_str().unwrap_or("");
    let job_id  = parse_uuid(job, "job_id");
    if archive.is_empty() || !std::path::Path::new(archive).exists() {
        tracing::error!(job = %job, "migrate_plesk: archive not found"); return;
    }
    let tmp = format!("/tmp/plesk-migrate-{}", job_id);
    let _ = std::fs::create_dir_all(&tmp);
    let ok = std::process::Command::new("tar").args(["-xf", archive, "-C", &tmp]).output()
        .map(|o| o.status.success()).unwrap_or(false);
    if !ok { tracing::error!("migrate_plesk: extraction failed"); let _ = std::fs::remove_dir_all(&tmp); return; }
    // Heuristic: detect domain directories in the dump.
    let mut domains = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&tmp) {
        for e in entries.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.contains('.') && e.path().is_dir() { domains.push(n); }
        }
    }
    tracing::warn!(job = %job, domains = ?domains,
        "migrate_plesk: archive extracted + domains detected. Automatic site/db/mail recreation \
         is a follow-up — recreate detected domains via Create Site, or run a cPanel-format import.");
    let _ = std::fs::remove_dir_all(&tmp);
}

async fn dispatch_collect_stats(state: &Arc<AppState>) {
    if let Err(e) = monitor::collect_site_stats(state).await {
        tracing::error!(error = %e, "collect_stats failed");
    }
}

async fn dispatch_renew_ssl(state: &Arc<AppState>) {
    if let Err(e) = ssl::renew_expiring_certs(state).await {
        tracing::error!(error = %e, "renew_ssl failed");
    }
}

/// Dispatch a language runtime provisioning job to jotti-agent via gRPC.
///
/// The job payload (enqueued by POST /api/v1/sites/:site_id/runtime) contains:
///   - site_id, domain, unix_user, version, server_id
///
/// This handler calls the appropriate SetupXxxSite gRPC RPC on the target
/// jotti-agent and updates the site status on completion.
async fn dispatch_setup_runtime(
    state: &Arc<AppState>,
    job: &serde_json::Value,
    runtime: &str,
) {
    let site_id   = parse_uuid(job, "site_id");
    let domain    = job["domain"].as_str().unwrap_or("").to_string();
    let unix_user = job["unix_user"].as_str().unwrap_or("").to_string();
    let version   = job["version"].as_str().unwrap_or("").to_string();
    let server_id = job["server_id"].as_str().and_then(|s| Uuid::parse_str(s).ok());

    if site_id.is_nil() || domain.is_empty() || unix_user.is_empty() {
        tracing::error!(job = %job, "setup_{}_site: missing required fields", runtime);
        return;
    }

    // Delegate to provisioning service which holds the gRPC client
    if let Err(e) = provisioning::setup_runtime_site(
        state,
        site_id,
        &domain,
        &unix_user,
        runtime,
        &version,
        server_id,
    )
    .await
    {
        tracing::error!(
            site_id = %site_id,
            %runtime,
            error = %e,
            "setup_{}_site failed",
            runtime
        );
        let _ = sqlx::query!(
            "UPDATE sites SET status = 'error', updated_at = NOW() WHERE id = $1",
            site_id
        )
        .execute(&state.db)
        .await;
    } else {
        let _ = sqlx::query!(
            "UPDATE sites SET status = 'active', updated_at = NOW() WHERE id = $1",
            site_id
        )
        .execute(&state.db)
        .await;
        tracing::info!(site_id = %site_id, %runtime, "runtime provisioning complete");
    }
}

/// Provision a per-site Valkey instance (Step 4.5).
///
/// Writes /etc/jottiecp/valkey/SITEID.conf with:
///   - port: 6378 + (index % 1000)
///   - unixsocket: /run/orbit/valkey/SITEID.sock
///   - maxmemory: 32mb community / 256mb pro
///   - requirepass: random 32-char hex string
///
/// Then enqueues a start_valkey job for jotti-agent.
async fn dispatch_provision_valkey(state: &Arc<AppState>, job: &serde_json::Value) {
    let site_id   = parse_uuid(job, "site_id");
    let db_id     = parse_uuid(job, "db_id");
    let unix_user = job["unix_user"].as_str().unwrap_or("").to_string();

    if site_id.is_nil() || unix_user.is_empty() {
        tracing::error!(job = %job, "provision_valkey: missing required fields");
        return;
    }

    // Derive a stable port offset from the site_id tail (same algo as runtime.rs)
    let tail: u32 = site_id
        .to_string()
        .bytes()
        .rev()
        .take(4)
        .fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
    let port: u16 = 6378 + (tail % 1000) as u16;

    // Generate a random password
    let password = generate_random_hex(32);

    let maxmemory = if state.config.is_pro() { "256mb" } else { "32mb" };

    let socket_path = format!("/run/orbit/valkey/{}.sock", site_id);
    let conf_dir    = "/etc/jottiecp/valkey";
    let conf_path   = format!("{}/{}.conf", conf_dir, site_id);
    let log_path    = format!("/var/log/orbit/valkey/{}.log", site_id);

    let conf_content = format!(
        "# JottiCP Valkey config — site {site_id}\n\
         # MANAGED — do not edit manually\n\
         \n\
         port {port}\n\
         bind 127.0.0.1\n\
         unixsocket {socket_path}\n\
         unixsocketperm 660\n\
         \n\
         requirepass {password}\n\
         \n\
         maxmemory {maxmemory}\n\
         maxmemory-policy allkeys-lru\n\
         \n\
         # Persistence: disabled for cache-only usage\n\
         save \"\"\n\
         appendonly no\n\
         \n\
         # Logging\n\
         loglevel notice\n\
         logfile {log_path}\n\
         \n\
         # Security\n\
         protected-mode yes\n\
         rename-command FLUSHALL \"\"\n\
         rename-command CONFIG \"\"\n\
         rename-command DEBUG \"\"\n\
         rename-command MONITOR \"\"\n"
    );

    // Ensure directories exist
    for dir in &[conf_dir, "/run/orbit/valkey", "/var/log/orbit/valkey"] {
        if let Err(e) = std::fs::create_dir_all(dir) {
            tracing::error!(dir, error = %e, "provision_valkey: create dir failed");
            return;
        }
    }

    if let Err(e) = std::fs::write(&conf_path, &conf_content) {
        tracing::error!(conf_path, error = %e, "provision_valkey: write config failed");
        return;
    }

    // Store the connection URI in the database record
    let connection_uri = format!(
        "valkey://:{}@127.0.0.1:{}/0",
        password, port
    );

    let _ = sqlx::query!(
        "UPDATE user_databases
         SET status = 'active', connection_string = $1, port = $2, updated_at = NOW()
         WHERE id = $3",
        connection_uri,
        port as i32,
        db_id,
    )
    .execute(&state.db)
    .await;

    // Signal jotti-agent via job queue to start the Valkey service
    {
        use redis::AsyncCommands;
        let start_job = serde_json::json!({
            "type": "start_valkey",
            "site_id": site_id,
            "config_path": conf_path,
            "db_id": db_id,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", start_job.to_string()).await.unwrap_or(());
    }

    tracing::info!(site_id = %site_id, port, "Valkey config written and start job enqueued");
}

/// Provision a per-site SurrealDB instance (Step 4.6).
///
/// Pro-only.  Writes /etc/jottiecp/surrealdb/SITEID.env and a systemd unit
/// orbit-surrealdb-SITEID.service with 15-minute idle stop.
async fn dispatch_provision_surrealdb(state: &Arc<AppState>, job: &serde_json::Value) {
    if !state.config.is_pro() {
        tracing::warn!("provision_surrealdb: not a Pro instance — skipping");
        return;
    }

    let site_id  = parse_uuid(job, "site_id");
    let db_id    = parse_uuid(job, "db_id");
    let db_name  = job["db_name"].as_str().unwrap_or("").to_string();
    let db_user  = job["db_user"].as_str().unwrap_or("").to_string();

    if site_id.is_nil() || db_name.is_empty() {
        tracing::error!(job = %job, "provision_surrealdb: missing required fields");
        return;
    }

    // Derive port: 5800 + hash of site_id tail
    let tail: u32 = site_id
        .to_string()
        .bytes()
        .rev()
        .take(4)
        .fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
    let port: u16 = 5800 + (tail % 500) as u16;

    let surreal_pass = generate_random_hex(24);
    let data_path    = format!("/var/jottiecp/surrealdb/{}", site_id);
    let env_dir      = "/etc/jottiecp/surrealdb";
    let env_path     = format!("{}/{}.env", env_dir, site_id);

    let env_content = format!(
        "# JottiCP SurrealDB env — site {site_id}\n\
         SURREAL_PATH={data_path}/rocksdb\n\
         SURREAL_USER=root\n\
         SURREAL_PASS={surreal_pass}\n\
         SURREAL_BIND=127.0.0.1:{port}\n\
         SURREAL_LOG=warn\n"
    );

    // Create required directories
    for dir in &[env_dir, &data_path, &format!("{}/rocksdb", data_path)] {
        if let Err(e) = std::fs::create_dir_all(dir) {
            tracing::error!(dir, error = %e, "provision_surrealdb: create dir failed");
            return;
        }
    }

    if let Err(e) = std::fs::write(&env_path, &env_content) {
        tracing::error!(env_path, error = %e, "provision_surrealdb: write env failed");
        return;
    }

    // Write systemd unit with 15-minute inactivity stop
    let unit_name    = format!("orbit-surrealdb-{}", site_id);
    let unit_path    = format!("/etc/systemd/system/{}.service", unit_name);
    let surreal_bin  = "/usr/local/bin/surreal";

    let unit_content = format!(
        "[Unit]\n\
         Description=JottiCP SurrealDB — site {site_id}\n\
         After=network.target\n\
         StopWhenUnneeded=yes\n\
         \n\
         [Service]\n\
         Type=simple\n\
         EnvironmentFile={env_path}\n\
         ExecStart={surreal_bin} start \
             --bind 127.0.0.1:{port} \
             --user ${{SURREAL_USER}} \
             --pass ${{SURREAL_PASS}} \
             --log ${{SURREAL_LOG}} \
             rocksdb:${{SURREAL_PATH}}\n\
         Restart=on-failure\n\
         RestartSec=5s\n\
         # Idle stop after 15 minutes — SurrealDB is spun up on demand\n\
         RuntimeMaxSec=900\n\
         \n\
         # Security\n\
         NoNewPrivileges=yes\n\
         PrivateTmp=yes\n\
         ProtectHome=yes\n\
         ReadWritePaths={data_path}\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n"
    );

    if let Err(e) = std::fs::write(&unit_path, &unit_content) {
        tracing::error!(unit_path, error = %e, "provision_surrealdb: write unit failed");
        return;
    }

    // daemon-reload and enable (but don't start yet — lazy start on first connection)
    let _ = std::process::Command::new("systemctl")
        .args(["daemon-reload"])
        .output();
    let _ = std::process::Command::new("systemctl")
        .args(["enable", &format!("{}.service", unit_name)])
        .output();

    // Store connection details
    let connection_uri = format!(
        "surreal://root:{}@127.0.0.1:{}/{}",
        surreal_pass, port, db_name
    );

    let _ = sqlx::query!(
        "UPDATE user_databases
         SET status = 'active', connection_string = $1, port = $2, updated_at = NOW()
         WHERE id = $3",
        connection_uri,
        port as i32,
        db_id,
    )
    .execute(&state.db)
    .await;

    tracing::info!(
        site_id = %site_id,
        port,
        unit = %unit_name,
        "SurrealDB provisioned (lazy start — activates on first connection)"
    );
}

/// Dispatch a cPanel migration job (Step 4.15).
///
/// Job payload fields:
///   - job_id:         UUID of the migration_jobs row
///   - archive_path:   absolute path to the uploaded .tar.gz
///   - target_site_id: optional UUID (may be null/absent)
///   - user_id:        UUID of the requesting user
async fn dispatch_cpanel_migration(state: &Arc<AppState>, job: &serde_json::Value) {
    let job_id   = parse_uuid(job, "job_id");
    let archive  = job["archive_path"].as_str().unwrap_or("").to_string();
    let site_id  = job["target_site_id"].as_str().and_then(|s| Uuid::parse_str(s).ok());

    if job_id.is_nil() || archive.is_empty() {
        tracing::error!(job = %job, "cpanel_migration: missing required fields (job_id, archive_path)");
        return;
    }

    let archive_path = std::path::Path::new(&archive);

    if let Err(e) = migration::process_cpanel_migration(state, job_id, archive_path, site_id).await {
        tracing::error!(
            job_id = %job_id,
            error = %e,
            "cpanel_migration failed"
        );

        // Mark job as failed with error in report
        let _ = sqlx::query!(
            r#"UPDATE migration_jobs
               SET status = 'failed', progress = 0,
                   report = $1, completed_at = NOW()
               WHERE id = $2"#,
            serde_json::json!({ "error": e.to_string() }),
            job_id
        )
        .execute(&state.db)
        .await;
    }
}

// ── start_valkey dispatcher ───────────────────────────────────────────────────

/// Forward the start_valkey job to jotti-agent (or start locally if no server_id).
///
/// jotti-agent is responsible for executing `valkey-server /etc/jottiecp/valkey/SITEID.conf`
/// inside the site's cgroup slice.  This dispatcher simply relays the config path.
async fn dispatch_start_valkey(state: &Arc<AppState>, job: &serde_json::Value) {
    let site_id     = parse_uuid(job, "site_id");
    let config_path = job["config_path"].as_str().unwrap_or("").to_string();
    let server_id   = job["server_id"].as_str().and_then(|s| Uuid::parse_str(s).ok());

    if site_id.is_nil() || config_path.is_empty() {
        tracing::error!(job = %job, "start_valkey: missing required fields (site_id, config_path)");
        return;
    }

    // Config path must be within the expected directory to prevent injection.
    if !config_path.starts_with("/etc/jottiecp/valkey/") {
        tracing::error!(
            config_path = %config_path,
            "start_valkey: config_path outside expected directory — refusing"
        );
        return;
    }

    tracing::info!(site_id = %site_id, config_path = %config_path, "Dispatching start_valkey to jotti-agent");

    let agent_addr = if let Some(sid) = server_id {
        match sqlx::query!(
            "SELECT agent_address FROM servers WHERE id = $1 AND deleted_at IS NULL",
            sid
        )
        .fetch_optional(&state.db)
        .await
        {
            Ok(Some(row)) => row.agent_address.unwrap_or_else(|| "127.0.0.1:7443".into()),
            _ => "127.0.0.1:7443".into(),
        }
    } else {
        std::env::var("ORBIT_AGENT_LOCAL_ADDR").unwrap_or_else(|_| "127.0.0.1:7443".into())
    };

    let body = serde_json::json!({
        "site_id":     site_id,
        "config_path": config_path,
    });

    match state.http
        .post(format!("http://{}/agent/start_valkey", agent_addr))
        .json(&body)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            tracing::info!(site_id = %site_id, "start_valkey OK");
        }
        Ok(resp) => {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            tracing::error!(site_id = %site_id, %status, body = %text, "start_valkey agent returned error");
        }
        Err(e) => {
            tracing::error!(site_id = %site_id, error = %e, "start_valkey agent call failed");
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_uuid(job: &serde_json::Value, field: &str) -> Uuid {
    job[field]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
        .unwrap_or_else(Uuid::nil)
}

/// Generate a cryptographically random lowercase hex string of `byte_len` bytes
/// (resulting string length = 2 × byte_len).
///
/// Uses the `rand` crate's `OsRng` which reads from the OS CSPRNG
/// (`getrandom` syscall on Linux).
fn generate_random_hex(byte_len: usize) -> String {
    use rand::RngCore;
    let mut bytes = vec![0u8; byte_len];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}


// ── App installer helpers ─────────────────────────────────────────────────────

/// Look up UID/GID for a Unix username by parsing /etc/passwd.
fn uid_gid_for_user(username: &str) -> Option<(u32, u32)> {
    let content = std::fs::read_to_string("/etc/passwd").ok()?;
    for line in content.lines() {
        let parts: Vec<&str> = line.splitn(7, ':').collect();
        if parts.len() >= 4 && parts[0] == username {
            let uid: u32 = parts[2].parse().ok()?;
            let gid: u32 = parts[3].parse().ok()?;
            return Some((uid, gid));
        }
    }
    None
}

/// Substitute `{{key}}` placeholders in a string with values from a map.
fn subst(template: &str, vars: &std::collections::HashMap<String, String>) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{{{}}}}}", k), v);
    }
    out
}

/// Load the raw TOML for a specific app from `ORBIT_APPS_DIR`.
fn load_app_toml_raw(app_id: &str) -> Option<toml::Value> {
    let apps_dir = std::env::var("ORBIT_APPS_DIR")
        .unwrap_or_else(|_| "/opt/jottiecp/apps".to_string());
    let dir = std::fs::read_dir(&apps_dir).ok()?;
    for entry in dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") { continue; }
        let content = match std::fs::read_to_string(&path) { Ok(c) => c, Err(_) => continue };
        let table: toml::Value = match toml::from_str(&content) { Ok(v) => v, Err(_) => continue };
        let id = table.get("app").and_then(|a| a.get("id")).and_then(|v| v.as_str()).unwrap_or("");
        if id == app_id { return Some(table); }
    }
    None
}

// ── App installer dispatchers ─────────────────────────────────────────────────

/// Dispatch an `install_app` job.
///
/// Reads `[[steps]]` from the app's TOML manifest and executes them in order.
/// Supports `cli` steps (subprocess) and `permissions` steps (chown/chmod).
async fn dispatch_install_app(state: &Arc<AppState>, job: &serde_json::Value) {
    use redis::AsyncCommands;
    use std::collections::HashMap;

    let install_id = parse_uuid(job, "install_id");
    let site_id    = parse_uuid(job, "site_id");
    let app_id     = job["app_id"].as_str().unwrap_or("").to_string();
    let domain     = job["domain"].as_str().unwrap_or("").to_string();
    let params     = job["params"].clone();

    if install_id.is_nil() || site_id.is_nil() || app_id.is_empty() {
        tracing::error!(job = %job, "install_app: missing required fields");
        return;
    }

    let channel = format!("orbit:install-progress:{install_id}");

    macro_rules! publish_progress {
        ($step:expr, $pct:expr, $msg:expr) => {{
            let payload = serde_json::json!({
                "step": $step,
                "pct":  $pct,
                "msg":  $msg,
            })
            .to_string();
            let mut conn = state.valkey.clone();
            let _ = conn.publish::<_, _, ()>(&channel, &payload).await;
        }};
    }

    macro_rules! publish_error {
        ($msg:expr) => {{
            let payload = serde_json::json!({
                "step": "error",
                "pct":  0,
                "msg":  $msg,
            })
            .to_string();
            let mut conn = state.valkey.clone();
            let _ = conn.publish::<_, _, ()>(&channel, &payload).await;
        }};
    }

    // Mark as installing
    let mark_result = sqlx::query!(
        "UPDATE app_installs SET status = 'installing' WHERE id = $1 AND status = 'pending'",
        install_id
    )
    .execute(&state.db)
    .await;

    if let Err(e) = mark_result {
        tracing::error!(install_id = %install_id, error = %e, "install_app: failed to mark installing");
        return;
    }

    publish_progress!("start", 5, format!("Starting installation of {} on {}", app_id, domain));

    // Load site info
    let site_row = sqlx::query!(
        "SELECT unix_user, service_port, php_version FROM sites WHERE id = $1 AND deleted_at IS NULL",
        site_id
    )
    .fetch_optional(&state.db)
    .await;

    let site_row = match site_row {
        Ok(Some(r)) => r,
        Ok(None)    => {
            publish_error!(format!("Site {} not found", site_id));
            fail_install(state, install_id, "site not found", "").await;
            return;
        }
        Err(e) => {
            publish_error!(format!("DB error: {e}"));
            fail_install(state, install_id, &e.to_string(), "").await;
            return;
        }
    };

    let unix_user   = site_row.unix_user.clone();
    let php_version = site_row.php_version.clone();
    let public_html = format!("/home/{unix_user}/public_html");

    // Load TOML manifest
    let manifest = match load_app_toml_raw(&app_id) {
        Some(m) => m,
        None    => {
            publish_error!(format!("App manifest '{}' not found", app_id));
            fail_install(state, install_id, "manifest not found", "").await;
            return;
        }
    };

    // Build template variable map
    let mut vars: HashMap<String, String> = HashMap::new();
    vars.insert("unix_user".into(),   unix_user.clone());
    vars.insert("domain".into(),      domain.clone());
    vars.insert("public_html".into(), public_html.clone());
    vars.insert("php_version".into(), php_version.clone());

    // Secrets 1-8 (for WP salts, Laravel APP_KEY, etc.)
    for i in 1..=8u8 {
        vars.insert(format!("secret_{i}"), generate_random_hex(32));
    }

    // Params from wizard (admin credentials, site_title, etc.)
    let admin_user     = params["admin_user"].as_str().unwrap_or("admin").to_string();
    let admin_email    = params["admin_email"].as_str().unwrap_or("admin@localhost").to_string();
    let admin_password = params["admin_password"]
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| generate_random_hex(12));
    let site_title     = params["site_title"].as_str().unwrap_or(&domain).to_string();

    vars.insert("admin_user".into(),     admin_user.clone());
    vars.insert("admin_email".into(),    admin_email.clone());
    vars.insert("admin_password".into(), admin_password.clone());
    vars.insert("site_title".into(),     site_title.clone());
    vars.insert("memory_limit".into(),   "256M".into());

    // Generate MySQL credentials if the manifest requires a database
    let requires_db = manifest
        .get("app")
        .and_then(|a| a.get("requires_db"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let toml_db_type = manifest
        .get("database")
        .and_then(|d| d.get("db_type"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let needs_mysql = !requires_db.is_empty() || !toml_db_type.is_empty();

    let mut db_name     = String::new();
    let mut db_user_str = String::new();
    let mut db_password = String::new();

    if needs_mysql {
        let user_short = &unix_user[..unix_user.len().min(12)];
        let app_short  = &app_id[..app_id.len().min(8)];
        db_name     = format!("{user_short}_{app_short}");
        db_user_str = format!("orb_{user_short}_{app_short}");
        db_password = generate_random_hex(16);

        vars.insert("db_name".into(),     db_name.clone());
        vars.insert("db_user".into(),     db_user_str.clone());
        vars.insert("db_password".into(), db_password.clone());

        publish_progress!("db", 15, format!("Creating database '{}'", db_name));

        let create_sql = format!(
            "CREATE DATABASE IF NOT EXISTS `{db_name}` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci; \
             CREATE USER IF NOT EXISTS '{db_user_str}'@'%' IDENTIFIED BY '{db_password}'; \
             GRANT ALL PRIVILEGES ON `{db_name}`.* TO '{db_user_str}'@'%'; \
             FLUSH PRIVILEGES;"
        );
        let db_result = tokio::process::Command::new("mysql")
            .arg("-e")
            .arg(&create_sql)
            .output()
            .await;

        match db_result {
            Ok(out) if out.status.success() => {
                tracing::info!(db = %db_name, user = %db_user_str, "install_app: MySQL DB created");
            }
            Ok(out) => {
                let err = String::from_utf8_lossy(&out.stderr);
                publish_error!(format!("Failed to create database: {}", err));
                fail_install(state, install_id, &format!("mysql create db: {}", err), "").await;
                return;
            }
            Err(e) => {
                publish_error!(format!("mysql command failed: {e}"));
                fail_install(state, install_id, &e.to_string(), "").await;
                return;
            }
        }

        // Store db_name in metadata for cleanup on failure
        let _ = sqlx::query!(
            "UPDATE app_installs SET metadata = jsonb_set(COALESCE(metadata, '{}'), '{db_name}', $2) WHERE id = $1",
            install_id,
            serde_json::Value::String(db_name.clone())
        )
        .execute(&state.db)
        .await;
    }

    // ── Download + extract the app archive ([download] + [extract]) ──────────────
    // The engine previously skipped these sections, so app source (e.g. WordPress core)
    // was never fetched → wp-cli steps failed ("Pass --path / wp core download"). Fetch +
    // verify sha256 + extract (strip_components) into dest_dir BEFORE config/steps.
    if let Some(dl) = manifest.get("download") {
        if let Some(url) = dl.get("url").and_then(|v| v.as_str()) {
            let url = url.to_string();
            let expect_sha = dl.get("sha256").and_then(|v| v.as_str()).map(|s| s.to_lowercase());
            publish_progress!("download", 10, format!("Downloading {url}"));
            let bytes = match reqwest::get(&url).await {
                Ok(r) => match r.bytes().await {
                    Ok(b) => b.to_vec(),
                    Err(e) => { publish_error!(format!("download read failed: {e}")); fail_install(state, install_id, &format!("download read failed: {e}"), "").await; return; }
                },
                Err(e) => { publish_error!(format!("download failed: {e}")); fail_install(state, install_id, &format!("download failed: {e}"), "").await; return; }
            };
            if let Some(exp) = &expect_sha {
                use sha2::{Digest, Sha256};
                let got = format!("{:x}", Sha256::digest(&bytes));
                if &got != exp {
                    let m = format!("sha256 mismatch (expected {exp}, got {got})");
                    publish_error!(m.clone()); fail_install(state, install_id, &m, "").await; return;
                }
            }
            let (dest_dir, strip) = manifest.get("extract").map(|ex| (
                ex.get("dest_dir").and_then(|v| v.as_str()).unwrap_or("public_html").to_string(),
                ex.get("strip_components").and_then(|v| v.as_integer()).unwrap_or(0) as usize,
            )).unwrap_or_else(|| ("public_html".to_string(), 0));
            let dest = format!("/home/{unix_user}/{dest_dir}");
            publish_progress!("extract", 18, "Extracting archive");
            let dest_c = dest.clone();
            let res = tokio::task::spawn_blocking(move || -> Result<(), String> {
                use flate2::read::GzDecoder;
                let mut ar = tar::Archive::new(GzDecoder::new(&bytes[..]));
                std::fs::create_dir_all(&dest_c).map_err(|e| e.to_string())?;
                for entry in ar.entries().map_err(|e| e.to_string())? {
                    let mut e = entry.map_err(|e| e.to_string())?;
                    let p = e.path().map_err(|e| e.to_string())?.into_owned();
                    let comps: Vec<_> = p.components().collect();
                    if comps.len() <= strip { continue; }
                    let rel: std::path::PathBuf = comps[strip..].iter().collect();
                    if rel.as_os_str().is_empty()
                        || rel.components().any(|c| matches!(c, std::path::Component::ParentDir)) { continue; }
                    let out = std::path::Path::new(&dest_c).join(&rel);
                    e.unpack(&out).map_err(|e| e.to_string())?;
                }
                Ok(())
            }).await;
            match res {
                Ok(Ok(())) => {}
                Ok(Err(e)) => { publish_error!(format!("extract failed: {e}")); fail_install(state, install_id, &format!("extract failed: {e}"), "").await; return; }
                Err(e)     => { publish_error!(format!("extract task panicked: {e}")); fail_install(state, install_id, &format!("extract task: {e}"), "").await; return; }
            }
            let _ = run_cmd("chown", &["-R", &format!("{unix_user}:{unix_user}"), &dest]);
        }
    }

    // Write config file from [config] template if present
    if let Some(config_section) = manifest.get("config") {
        if let (Some(config_path_raw), Some(template_raw)) = (
            config_section.get("config_path").and_then(|v| v.as_str()),
            config_section.get("template").and_then(|v| v.as_str()),
        ) {
            let config_path    = format!("/home/{unix_user}/{}", subst(config_path_raw, &vars));
            let config_content = subst(template_raw, &vars);

            publish_progress!("config", 20, "Writing configuration file");

            if let Some(parent) = std::path::Path::new(&config_path).parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            if let Err(e) = std::fs::write(&config_path, &config_content) {
                tracing::warn!(path = %config_path, error = %e, "install_app: write config (non-fatal)");
            }
        }
    }

    // Execute [[steps]] from the manifest
    let steps = manifest
        .get("steps")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let total_steps = steps.len() as u32;

    for (i, step) in steps.iter().enumerate() {
        let step_pct  = 25 + ((i as u32) * 60 / total_steps.max(1));
        let step_type = step.get("type").and_then(|v| v.as_str()).unwrap_or("unknown");

        match step_type {
            "cli" => {
                let cmd_raw   = step.get("command").and_then(|v| v.as_str()).unwrap_or("");
                let cmd       = subst(cmd_raw, &vars);
                let run_as    = subst(step.get("run_as").and_then(|v| v.as_str()).unwrap_or("root"), &vars);
                let timeout_s = step.get("timeout_s").and_then(|v| v.as_integer()).unwrap_or(120) as u64;

                let args: Vec<String> = step
                    .get("args")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|a| a.as_str()).map(|a| subst(a, &vars)).collect())
                    .unwrap_or_default();

                publish_progress!("run", step_pct, format!("Step {}/{}: {}", i + 1, total_steps, cmd));
                tracing::info!(step = i + 1, cmd = %cmd, args = ?args, run_as = %run_as, "install_app: running step");

                // Drop privileges via CommandExt::uid/gid — jotti-panel runs as root
                // so it can call setuid/setgid in the child process. This avoids sudo/runuser
                // (which fail due to PAM/seccomp restrictions in the systemd service).
                #[allow(unused_imports)]
                use std::os::unix::process::CommandExt as _;
                let mut command = tokio::process::Command::new(&cmd);
                command.args(&args);
                command.env("HOME", format!("/home/{run_as}"));
                command.env("USER",    &run_as);
                command.env("LOGNAME", &run_as);
                if let Some((uid, gid)) = uid_gid_for_user(&run_as) {
                    command.uid(uid).gid(gid);
                }

                let result = tokio::time::timeout(
                    Duration::from_secs(timeout_s),
                    command.output(),
                )
                .await;

                match result {
                    Ok(Ok(out)) if out.status.success() => {
                        tracing::info!(step = i + 1, "install_app: step succeeded");
                    }
                    Ok(Ok(out)) => {
                        let stderr  = String::from_utf8_lossy(&out.stderr);
                        let stdout  = String::from_utf8_lossy(&out.stdout);
                        let err_msg = format!(
                            "Step {}/{} failed (exit {}): {}",
                            i + 1, total_steps, out.status,
                            stderr.lines().last().unwrap_or(stdout.lines().last().unwrap_or(""))
                        );
                        tracing::error!(step = i + 1, stderr = %stderr, "install_app: step failed");
                        publish_error!(&err_msg);
                        fail_install(state, install_id, &err_msg, &db_name).await;
                        return;
                    }
                    Ok(Err(e)) => {
                        let err_msg = format!("Step {}/{} spawn failed: {e}", i + 1, total_steps);
                        publish_error!(&err_msg);
                        fail_install(state, install_id, &err_msg, &db_name).await;
                        return;
                    }
                    Err(_) => {
                        let err_msg = format!("Step {}/{} timed out after {}s", i + 1, total_steps, timeout_s);
                        publish_error!(&err_msg);
                        fail_install(state, install_id, &err_msg, &db_name).await;
                        return;
                    }
                }
            }

            "permissions" => {
                let path_raw  = step.get("path").and_then(|v| v.as_str()).unwrap_or("");
                let path      = subst(path_raw, &vars);
                let owner     = subst(step.get("owner").and_then(|v| v.as_str()).unwrap_or("root"), &vars);
                let group     = subst(step.get("group").and_then(|v| v.as_str()).unwrap_or("www-data"), &vars);
                let dir_mode  = step.get("dir_mode").and_then(|v| v.as_str()).unwrap_or("0755");
                let file_mode = step.get("file_mode").and_then(|v| v.as_str()).unwrap_or("0644");

                publish_progress!("run", step_pct, format!("Step {}/{}: permissions on {}", i + 1, total_steps, path));

                let _ = tokio::process::Command::new("chown")
                    .args(["-R", &format!("{owner}:{group}"), &path])
                    .output().await;
                let _ = tokio::process::Command::new("find")
                    .args([&path, "-type", "d", "-exec", "chmod", dir_mode, "{}", ";"])
                    .output().await;
                let _ = tokio::process::Command::new("find")
                    .args([&path, "-type", "f", "-exec", "chmod", file_mode, "{}", ";"])
                    .output().await;
            }

            other => {
                tracing::warn!(step = i + 1, step_type = other, "install_app: unknown step type — skipping");
            }
        }
    }

    publish_progress!("finalize", 90, "Installation succeeded, storing credentials");

    let stored_meta = serde_json::json!({
        "admin_user":     admin_user,
        "admin_email":    admin_email,
        "admin_password": admin_password,
        "db_name":        db_name,
        "db_user":        db_user_str,
        "db_password":    db_password,
    });

    let admin_url = manifest
        .get("post_install")
        .and_then(|p| p.get("admin_url"))
        .and_then(|v| v.as_str())
        .map(|u| subst(u, &vars))
        .unwrap_or_else(|| format!("https://{domain}"));

    let _ = sqlx::query!(
        r#"UPDATE app_installs
           SET status = 'active', installed_at = NOW(),
               metadata = $2, installed_version = 'latest'
           WHERE id = $1"#,
        install_id,
        stored_meta
    )
    .execute(&state.db)
    .await;

    let done_payload = serde_json::json!({
        "step":           "done",
        "pct":            100,
        "status":         "active",
        "admin_url":      admin_url,
        "admin_user":     admin_user,
        "admin_password": admin_password,
        "admin_email":    admin_email,
        "msg":            format!("{} installed successfully on {}", app_id, domain),
    })
    .to_string();

    let mut conn = state.valkey.clone();
    let _ = conn.publish::<_, _, ()>(&channel, &done_payload).await;

    tracing::info!(install_id = %install_id, app_id = %app_id, domain = %domain, "App installation complete");
}

/// Mark an install as failed and drop the MySQL DB if one was created.
async fn fail_install(state: &Arc<AppState>, install_id: Uuid, reason: &str, db_name: &str) {
    let error_params = serde_json::json!({ "error": reason });
    let _ = sqlx::query!(
        "UPDATE app_installs SET status = 'failed', metadata = $2 WHERE id = $1",
        install_id, error_params
    )
    .execute(&state.db)
    .await;

    // Drop MySQL database if one was provisioned
    let db_to_drop = if !db_name.is_empty() {
        db_name.to_string()
    } else {
        // Fall back to stored metadata
        sqlx::query_scalar!(
            "SELECT metadata->>'db_name' FROM app_installs WHERE id = $1",
            install_id
        )
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
        .flatten()
        .unwrap_or_default()
    };

    if !db_to_drop.is_empty() {
        let safe_name = db_to_drop.replace('`', "").replace('"', "");
        // Drop the DB and the app-specific user (user name = "orb_" + db_name without prefix)
        let db_user_to_drop = format!("orb_{safe_name}");
        let drop_sql  = format!(
            "DROP DATABASE IF EXISTS `{safe_name}`; \
             DROP USER IF EXISTS '{db_user_to_drop}'@'%';"
        );
        let result    = tokio::process::Command::new("mysql")
            .arg("-e").arg(&drop_sql)
            .output().await;
        match result {
            Ok(out) if out.status.success() => {
                tracing::info!(install_id = %install_id, db = %safe_name, "cleanup: dropped MySQL DB after install failure");
            }
            Ok(out) => {
                tracing::warn!(
                    install_id = %install_id,
                    db = %safe_name,
                    stderr = %String::from_utf8_lossy(&out.stderr),
                    "cleanup: failed to drop MySQL DB (non-fatal)"
                );
            }
            Err(e) => {
                tracing::warn!(install_id = %install_id, error = %e, "cleanup: mysql command failed (non-fatal)");
            }
        }
    }
}

/// Dispatch an `update_app` job (stubbed — triggers re-install from latest archive).
async fn dispatch_update_app(state: &Arc<AppState>, job: &serde_json::Value) {
    let install_id = parse_uuid(job, "install_id");
    let app_id     = job["app_id"].as_str().unwrap_or("").to_string();

    // For now: mark the installation status as 'updating' in the DB and log.
    // Full update logic (download → verify → swap files) follows the same
    // pattern as install_app but preserves existing config/data.
    let _ = sqlx::query!(
        "UPDATE app_installs SET status = 'updating' WHERE id = $1 AND status = 'active'",
        install_id
    )
    .execute(&state.db)
    .await;

    tracing::info!(
        install_id = %install_id,
        app_id = %app_id,
        "update_app: queued (full update logic pending)"
    );

    // TODO: implement full update pipeline:
    //  1. Download latest archive from manifest URL.
    //  2. Verify Ed25519 signature.
    //  3. Extract to staging dir, run migrations, swap symlink.
    //  4. Set status = 'active' on success; fail_install() on failure.
}

// ── Email provisioning ────────────────────────────────────────────────────────

async fn dispatch_provision_email(state: &Arc<AppState>, job: &serde_json::Value) {
    let account_id = job["account_id"].as_str().unwrap_or("").to_string();
    let address    = job["address"].as_str().unwrap_or("").to_string();
    let password   = job["password"].as_str().unwrap_or("").to_string();

    if address.is_empty() || password.is_empty() {
        tracing::error!(job = %job, "provision_email_account: missing address or password");
        return;
    }

    match email_provision_dovecot(&address, &password).await {
        Ok(()) => {
            let _ = email_sync_sogo(&address, &password).await;
            if let Ok(id) = Uuid::parse_str(&account_id) {
                let _ = sqlx::query!(
                    "UPDATE email_accounts SET status = 'active' WHERE id = $1",
                    id
                )
                .execute(&state.db)
                .await;
            }
            tracing::info!(address = %address, "Email account provisioned");
        }
        Err(e) => tracing::error!(address = %address, error = %e, "provision_email_account failed"),
    }
}

async fn dispatch_change_email_password(state: &Arc<AppState>, job: &serde_json::Value) {
    let account_id = job["account_id"].as_str().unwrap_or("").to_string();
    let address    = job["address"].as_str().unwrap_or("").to_string();
    let password   = job["new_password"].as_str().unwrap_or("").to_string();

    if address.is_empty() || password.is_empty() {
        tracing::error!(job = %job, "change_email_password: missing fields");
        return;
    }

    // Dovecot authenticates mail via the SQL passdb (email_accounts.password_hash, argon2),
    // NOT the /etc/dovecot/users flat file — so the change MUST update that column, using the
    // SAME hashing as account creation. (Previously only the unused flat file was written, so
    // password changes never took effect for SMTP/IMAP auth.)
    let new_hash = match crate::api::auth::hash_password(&password) {
        Ok(h) => h,
        Err(e) => { tracing::error!(address = %address, error = %e, "change_email_password: hash failed"); return; }
    };
    let upd = if let Ok(aid) = account_id.parse::<Uuid>() {
        sqlx::query!("UPDATE email_accounts SET password_hash = $1 WHERE id = $2 AND deleted_at IS NULL", new_hash, aid)
            .execute(&state.db).await
    } else {
        sqlx::query!("UPDATE email_accounts SET password_hash = $1 WHERE address = $2 AND deleted_at IS NULL", new_hash, address)
            .execute(&state.db).await
    };
    match upd {
        Ok(r) if r.rows_affected() > 0 => {
            // Best-effort: refresh flat-file + webmail SSO so any file-based consumer stays in sync.
            let _ = email_provision_dovecot(&address, &password).await;
            let _ = email_sync_sogo(&address, &password).await;
            tracing::info!(address = %address, "Email password changed (SQL passdb updated)");
        }
        Ok(_)  => tracing::error!(address = %address, "change_email_password: no matching account row"),
        Err(e) => tracing::error!(address = %address, error = %e, "change_email_password: db update failed"),
    }
}

async fn dispatch_deprovision_email(state: &Arc<AppState>, job: &serde_json::Value) {
    let account_id = job["account_id"].as_str().unwrap_or("").to_string();
    let address    = job["address"].as_str().unwrap_or("").to_string();

    if address.is_empty() {
        tracing::error!(job = %job, "deprovision_email_account: missing address");
        return;
    }

    // Remove from dovecot users file
    if let Err(e) = email_remove_dovecot(&address).await {
        tracing::error!(address = %address, error = %e, "deprovision_email: dovecot removal failed");
    }

    // Remove from sogo_users
    let sql = format!(
        "DELETE FROM sogo_users WHERE c_uid = '{}';",
        address.replace('\'', "''")
    );
    let _ = tokio::process::Command::new("psql")
        .args(["-h", "127.0.0.1", "-U", "sogo", "-d", "sogo", "-c", &sql])
        .env("PGPASSWORD", "sogo_pass")
        .output()
        .await;

    if let Ok(id) = Uuid::parse_str(&account_id) {
        let _ = sqlx::query!(
            "UPDATE email_accounts SET status = 'deleted', deleted_at = NOW() WHERE id = $1",
            id
        )
        .execute(&state.db)
        .await;
    }

    tracing::info!(address = %address, "Email account deprovisioned");
}

// ── Email helpers ─────────────────────────────────────────────────────────────

async fn email_provision_dovecot(address: &str, password: &str) -> anyhow::Result<()> {
    let parts: Vec<&str> = address.splitn(2, '@').collect();
    anyhow::ensure!(parts.len() == 2, "invalid email address");
    let local  = parts[0];
    let domain = parts[1];

    // Generate SHA512-CRYPT hash via openssl
    let hash = {
        use rand::Rng;
        let chars = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let salt: String = (0..16)
            .map(|_| chars[rand::thread_rng().gen_range(0..chars.len())] as char)
            .collect();
        let out = tokio::process::Command::new("openssl")
            .args(["passwd", "-6", "-salt", &salt, password])
            .output()
            .await?;
        anyhow::ensure!(out.status.success(), "openssl passwd failed: {:?}", out.stderr);
        String::from_utf8(out.stdout)?.trim().to_string()
    };

    // Ensure mail directory exists with correct ownership
    let mail_dir = format!("/var/mail/vhosts/{}/{}", domain, local);
    tokio::fs::create_dir_all(&mail_dir).await?;
    tokio::process::Command::new("chown")
        .args(["-R", "vmail:vmail", &mail_dir])
        .status()
        .await?;

    // Atomically update /etc/dovecot/users
    let entry = format!(
        "{}:{{SHA512-CRYPT}}{}:::{}\n",
        address, hash, mail_dir
    );
    email_update_passwd_file("/etc/dovecot/users", address, &entry).await?;

    // Reload dovecot auth without full restart
    tokio::process::Command::new("doveadm")
        .args(["reload"])
        .status()
        .await
        .ok();

    Ok(())
}

async fn email_remove_dovecot(address: &str) -> anyhow::Result<()> {
    email_update_passwd_file("/etc/dovecot/users", address, "").await
}

async fn email_update_passwd_file(path: &str, address: &str, new_entry: &str) -> anyhow::Result<()> {
    let contents = tokio::fs::read_to_string(path).await.unwrap_or_default();

    let prefix = format!("{}:", address);
    let mut lines: Vec<&str> = contents
        .lines()
        .filter(|l| !l.starts_with(&prefix))
        .collect();

    if !new_entry.is_empty() {
        // new_entry already ends with \n; add as a str slice on the heap
        let trimmed = new_entry.trim_end_matches('\n');
        lines.push(trimmed);
    }

    let new_contents = if lines.is_empty() {
        String::new()
    } else {
        lines.join("\n") + "\n"
    };

    let tmp = format!("{}.tmp.{}", path, std::process::id());
    tokio::fs::write(&tmp, new_contents.as_bytes()).await?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}

async fn email_sync_sogo(address: &str, password: &str) -> anyhow::Result<()> {
    let cn       = address.split('@').next().unwrap_or(address);
    let addr_esc = address.replace('\'', "''");
    let pw_esc   = password.replace('\'', "''");
    let cn_esc   = cn.replace('\'', "''");

    let sql = format!(
        "INSERT INTO sogo_users (c_uid,c_name,c_cn,c_password,c_active,mail) \
         VALUES ('{a}','{a}','{cn}','{pw}',1,'{a}') \
         ON CONFLICT (c_uid) DO UPDATE \
         SET c_password=EXCLUDED.c_password, c_active=1, mail=EXCLUDED.mail;",
        a = addr_esc, cn = cn_esc, pw = pw_esc
    );

    let out = tokio::process::Command::new("psql")
        .args(["-h", "127.0.0.1", "-U", "sogo", "-d", "sogo", "-c", &sql])
        .env("PGPASSWORD", "sogo_pass")
        .output()
        .await?;

    anyhow::ensure!(out.status.success(), "sogo_users sync failed");
    Ok(())
}
