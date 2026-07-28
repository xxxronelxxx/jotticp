use crate::{AppState, error::ApiError};
use std::sync::Arc;
use uuid::Uuid;

/// gRPC client types generated from proto/jotti.proto at build time (see build.rs).
pub mod agentproto {     tonic::include_proto!("jotti"); }

pub struct ProvisioningJob {
    pub site_id:     Uuid,
    pub domain:      String,
    pub php_version: String,
    pub web_server:  String,
    pub unix_user:   String,
    pub server_id:   Option<Uuid>,
}

// ── Input validation ──────────────────────────────────────────────────────────

fn validate_domain(domain: &str) -> anyhow::Result<()> {
    let re = regex::Regex::new(r"^[a-zA-Z0-9][a-zA-Z0-9\-\.]{0,251}[a-zA-Z0-9]$").unwrap();
    anyhow::ensure!(re.is_match(domain), "Invalid domain: {}", domain);
    Ok(())
}

pub fn validate_domain_pub(domain: &str) -> anyhow::Result<()> { validate_domain(domain) }

fn validate_unix_user(user: &str) -> anyhow::Result<()> {
    let re = regex::Regex::new(r"^orbit_[a-z0-9_]{1,24}$").unwrap();
    anyhow::ensure!(re.is_match(user), "Invalid unix user: {}", user);
    Ok(())
}

pub fn validate_unix_user_pub(user: &str) -> anyhow::Result<()> { validate_unix_user(user) }

fn validate_php_version(version: &str) -> anyhow::Result<()> {
    let re = regex::Regex::new(r"^[78]\.[0-9]+$").unwrap();
    anyhow::ensure!(re.is_match(version), "Invalid PHP version: {}", version);
    Ok(())
}

fn validate_web_server(server: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        matches!(server, "nginx" | "ols" | "apache"),
        "Invalid web_server '{}'; must be nginx, ols, or apache",
        server
    );
    Ok(())
}

// ── Main entry point ──────────────────────────────────────────────────────────

pub async fn provision_site(state: &Arc<AppState>, job: ProvisioningJob) -> anyhow::Result<()> {
    // Validate all inputs up front to avoid any OS-level injection
    validate_domain(&job.domain)?;
    validate_unix_user(&job.unix_user)?;
    validate_php_version(&job.php_version)?;
    validate_web_server(&job.web_server)?;

    // Idempotency guard: if the site is already 'active' this is a job retry — skip cleanly.
    // Status 'error' is allowed through so that failed provisioning can be retried.
    let current_status = sqlx::query_scalar!(
        r#"SELECT status::text AS "status!" FROM sites WHERE id = $1"#,
        job.site_id
    )
    .fetch_optional(&state.db)
    .await?;

    if let Some(ref status) = current_status {
        if status == "active" {
            tracing::warn!(
                site_id = %job.site_id,
                "provision_site called for already-active site — skipping (idempotent retry)"
            );
            return Ok(());
        }
    }

    tracing::info!(
        site_id  = %job.site_id,
        domain   = %job.domain,
        user     = %job.unix_user,
        "Starting site provisioning"
    );

    let result = if let Some(server_id) = job.server_id {
        provision_via_agent(state, &job, server_id).await
    } else {
        provision_local(state, &job).await
    };

    match result {
        Ok(()) => {
            // Mark site active
            sqlx::query!(
                "UPDATE sites SET status = 'active', updated_at = NOW() WHERE id = $1",
                job.site_id
            )
            .execute(&state.db)
            .await?;

            // Queue SSL issuance
            {
                use redis::AsyncCommands;
                let ssl_job = serde_json::json!({
                    "type": "issue_ssl",
                    "site_id": job.site_id,
                    "domain": job.domain,
                    "challenge": "http-01",
                });
                let mut conn = state.valkey.clone();
                let _: () = conn.lpush("orbit:jobs", ssl_job.to_string()).await
                    .unwrap_or(());
            }

            // Write audit log
            write_audit_log(
                state,
                "site_provisioned",
                &job.site_id.to_string(),
                &format!("domain={} user={}", job.domain, job.unix_user),
            )
            .await;

            tracing::info!(site_id = %job.site_id, "Site provisioned successfully");
            Ok(())
        }
        Err(e) => {
            tracing::error!(site_id = %job.site_id, error = %e, "Provisioning failed");
            let _ = sqlx::query!(
                "UPDATE sites SET status = 'error', updated_at = NOW() WHERE id = $1",
                job.site_id
            )
            .execute(&state.db)
            .await;
            Err(e)
        }
    }
}

// ── Remote provisioning (via jotti-agent gRPC) ────────────────────────────────

/// Build an mTLS gRPC channel to an jotti-agent (certs from /etc/jottiecp/agent/).
pub async fn agent_channel(agent_addr: &str) -> anyhow::Result<tonic::transport::Channel> {
    let host = agent_addr.split(':').next().unwrap_or(agent_addr).to_string();
    let ca   = tokio::fs::read("/etc/jottiecp/agent/ca.pem").await
        .map_err(|e| anyhow::anyhow!("read agent CA: {}", e))?;
    let cert = tokio::fs::read("/etc/jottiecp/agent/panel-client.pem").await
        .map_err(|e| anyhow::anyhow!("read panel client cert: {}", e))?;
    let key  = tokio::fs::read("/etc/jottiecp/agent/panel-client-key.pem").await
        .map_err(|e| anyhow::anyhow!("read panel client key: {}", e))?;
    let tls = tonic::transport::ClientTlsConfig::new()
        .ca_certificate(tonic::transport::Certificate::from_pem(ca))
        .identity(tonic::transport::Identity::from_pem(cert, key))
        .domain_name(host);
    tonic::transport::Channel::from_shared(format!("https://{}", agent_addr))
        .map_err(|e| anyhow::anyhow!("Invalid agent address {}: {}", agent_addr, e))?
        .tls_config(tls).map_err(|e| anyhow::anyhow!("agent TLS config: {}", e))?
        .connect().await.map_err(|e| anyhow::anyhow!("Cannot connect to jotti-agent at {} over mTLS: {}", agent_addr, e))
}

/// Delete a site on a remote server via the agent.
pub async fn agent_delete_site(agent_addr: &str, domain: &str, delete_files: bool) -> anyhow::Result<()> {
    let ch = agent_channel(agent_addr).await?;
    let r = agentproto::agent_client::AgentClient::new(ch)
        .delete_site(agentproto::DeleteSiteRequest { domain: domain.to_string(), delete_files }).await
        .map_err(|e| anyhow::anyhow!("agent DeleteSite gRPC: {}", e))?.into_inner();
    if !r.success { anyhow::bail!("agent DeleteSite: {}", r.message); }
    Ok(())
}
/// Suspend a site on a remote server via the agent.
pub async fn agent_suspend_site(agent_addr: &str, domain: &str) -> anyhow::Result<()> {
    let ch = agent_channel(agent_addr).await?;
    let r = agentproto::agent_client::AgentClient::new(ch)
        .suspend_site(agentproto::SuspendSiteRequest { domain: domain.to_string() }).await
        .map_err(|e| anyhow::anyhow!("agent SuspendSite gRPC: {}", e))?.into_inner();
    if !r.success { anyhow::bail!("agent SuspendSite: {}", r.message); }
    Ok(())
}
/// Unsuspend a site on a remote server via the agent.
pub async fn agent_unsuspend_site(agent_addr: &str, domain: &str) -> anyhow::Result<()> {
    let ch = agent_channel(agent_addr).await?;
    let r = agentproto::agent_client::AgentClient::new(ch)
        .unsuspend_site(agentproto::UnsuspendSiteRequest { domain: domain.to_string() }).await
        .map_err(|e| anyhow::anyhow!("agent UnsuspendSite gRPC: {}", e))?.into_inner();
    if !r.success { anyhow::bail!("agent UnsuspendSite: {}", r.message); }
    Ok(())
}

/// Create a database on a remote server via the agent.
pub async fn agent_create_database(addr: &str, db_type: &str, db_name: &str, db_user: &str, db_password: &str) -> anyhow::Result<()> {
    let ch = agent_channel(addr).await?;
    let r = agentproto::agent_client::AgentClient::new(ch)
        .create_database(agentproto::CreateDatabaseRequest {
            db_type: db_type.into(), db_name: db_name.into(), db_user: db_user.into(), db_password: db_password.into(),
        }).await.map_err(|e| anyhow::anyhow!("agent CreateDatabase gRPC: {}", e))?.into_inner();
    if !r.success { anyhow::bail!("agent CreateDatabase: {}", r.error_msg); }
    Ok(())
}
/// Delete a database on a remote server via the agent.
pub async fn agent_delete_database(addr: &str, db_type: &str, db_name: &str, db_user: &str) -> anyhow::Result<()> {
    let ch = agent_channel(addr).await?;
    let r = agentproto::agent_client::AgentClient::new(ch)
        .delete_database(agentproto::DeleteDatabaseRequest {
            db_type: db_type.into(), db_name: db_name.into(), db_user: db_user.into(),
        }).await.map_err(|e| anyhow::anyhow!("agent DeleteDatabase gRPC: {}", e))?.into_inner();
    if !r.success { anyhow::bail!("agent DeleteDatabase: {}", r.message); }
    Ok(())
}
/// Create a DNS zone on a remote server via the agent.
pub async fn agent_create_dns_zone(addr: &str, zone: &str) -> anyhow::Result<()> {
    let z = format!("{}.", zone.trim_end_matches('.'));
    let ch = agent_channel(addr).await?;
    let r = agentproto::agent_client::AgentClient::new(ch)
        .create_dns_zone(agentproto::CreateDnsZoneRequest {
            zone: z.clone(), ns1: format!("ns1.{}", z), ns2: format!("ns2.{}", z),
            admin: format!("hostmaster@{}", zone.trim_end_matches('.')),
        }).await.map_err(|e| anyhow::anyhow!("agent CreateDnsZone gRPC: {}", e))?.into_inner();
    if !r.success { anyhow::bail!("agent CreateDnsZone: {}", r.message); }
    Ok(())
}
/// Delete a DNS zone on a remote server via the agent.
pub async fn agent_delete_dns_zone(addr: &str, zone: &str) -> anyhow::Result<()> {
    let ch = agent_channel(addr).await?;
    let r = agentproto::agent_client::AgentClient::new(ch)
        .delete_dns_zone(agentproto::DeleteDnsZoneRequest { zone: format!("{}.", zone.trim_end_matches('.')) })
        .await.map_err(|e| anyhow::anyhow!("agent DeleteDnsZone gRPC: {}", e))?.into_inner();
    if !r.success { anyhow::bail!("agent DeleteDnsZone: {}", r.message); }
    Ok(())
}
/// Switch PHP version for a site on a remote server via the agent (creates/swaps the FPM pool).
pub async fn agent_set_php_version(addr: &str, domain: &str, php_version: &str, unix_user: &str) -> anyhow::Result<()> {
    let ch = agent_channel(addr).await?;
    let r = agentproto::agent_client::AgentClient::new(ch)
        .create_php_fpm_pool(agentproto::PhpFpmPoolRequest {
            domain: domain.into(), php_version: php_version.into(), unix_user: unix_user.into(),
            socket_path: format!("/run/php/fpm-{}.sock", unix_user), max_children: 0, memory_limit_mb: 0,
        }).await.map_err(|e| anyhow::anyhow!("agent CreatePhpFpmPool gRPC: {}", e))?.into_inner();
    if !r.success { anyhow::bail!("agent CreatePhpFpmPool: {}", r.message); }
    Ok(())
}

/// Upsert a single DNS record on a remote server's PowerDNS via the agent.
pub async fn agent_upsert_dns_record(addr: &str, zone: &str, name: &str, rtype: &str, content: &str, ttl: i32, priority: i32) -> anyhow::Result<()> {
    let ch = agent_channel(addr).await?;
    let r = agentproto::agent_client::AgentClient::new(ch)
        .upsert_dns_record(agentproto::DnsRecordRequest {
            zone: format!("{}.", zone.trim_end_matches('.')), name: name.into(),
            rtype: rtype.into(), content: content.into(), ttl, priority,
        }).await.map_err(|e| anyhow::anyhow!("agent UpsertDnsRecord gRPC: {}", e))?.into_inner();
    if !r.success { anyhow::bail!("agent UpsertDnsRecord: {}", r.message); }
    Ok(())
}
/// Delete a single DNS record on a remote server's PowerDNS via the agent.
pub async fn agent_delete_dns_record(addr: &str, zone: &str, name: &str, rtype: &str) -> anyhow::Result<()> {
    let ch = agent_channel(addr).await?;
    let r = agentproto::agent_client::AgentClient::new(ch)
        .delete_dns_record(agentproto::DeleteDnsRecordRequest {
            zone: format!("{}.", zone.trim_end_matches('.')), name: name.into(), rtype: rtype.into(),
        }).await.map_err(|e| anyhow::anyhow!("agent DeleteDnsRecord gRPC: {}", e))?.into_inner();
    if !r.success { anyhow::bail!("agent DeleteDnsRecord: {}", r.message); }
    Ok(())
}

async fn provision_via_agent(
    state: &Arc<AppState>,
    job: &ProvisioningJob,
    server_id: Uuid,
) -> anyhow::Result<()> {
    // Look up the agent address for this server
    let server = sqlx::query!(
        "SELECT id, label, agent_address FROM servers WHERE id = $1 AND deleted_at IS NULL",
        server_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| anyhow::anyhow!("Server {} not found", server_id))?;

    let agent_addr = server.agent_address
        .ok_or_else(|| anyhow::anyhow!("Server {} has no agent address", server_id))?;

    tracing::info!(
        site_id   = %job.site_id,
        server_id = %server_id,
        agent     = %agent_addr,
        "Provisioning via jotti-agent gRPC"
    );

    // Connect to jotti-agent over MUTUAL TLS (gRPC) via the shared helper.
    let channel = agent_channel(&agent_addr).await?;
    let mut client = agentproto::agent_client::AgentClient::new(channel);
    let resp = client.create_site(agentproto::CreateSiteRequest {
        domain:        job.domain.clone(),
        php_version:   job.php_version.clone(),
        web_server:    job.web_server.clone(),
        disk_quota_mb: 0,
    })
    .await
    .map_err(|e| anyhow::anyhow!("jotti-agent CreateSite gRPC failed: {}", e))?
    .into_inner();

    if resp.status == "error" {
        anyhow::bail!("jotti-agent CreateSite error: {}", resp.error_msg);
    }
    tracing::info!(site_id = %job.site_id, server = %server.label, agent_user = %resp.unix_user,
        "jotti-agent CreateSite OK (gRPC mTLS)");
    Ok(())
}

// ── Local provisioning ────────────────────────────────────────────────────────

async fn provision_local(state: &Arc<AppState>, job: &ProvisioningJob) -> anyhow::Result<()> {
    tracing::info!(site_id = %job.site_id, "Running local provisioning");

    // Track which steps completed so we can roll back on failure.
    let mut unix_user_created  = false;
    let mut cgroup_written     = false;
    let mut php_fpm_written    = false;
    let mut vhost_written      = false;

    // ── Provision steps (sequential; track what completed for rollback) ────────

    // Step A: create unix user
    let result = create_unix_user(&job.unix_user);
    if result.is_err() {
        // Nothing to roll back yet
        return result;
    }
    unix_user_created = true;

    // Step B: create home directory
    let result = create_home_dir(&job.unix_user);
    if let Err(e) = result {
        // Rollback A
        let _ = run_cmd("userdel", &["--remove", &job.unix_user]);
        return Err(e);
    }

    // Step C: write cgroup slice + start it
    let result = write_cgroup_slice(&job.domain, &job.unix_user)
        .and_then(|_| run_cmd("systemctl", &["daemon-reload"]))
        .and_then(|_| run_cmd("systemctl", &["start", &format!("website-{}.slice", job.domain)]));
    if let Err(e) = result {
        let _ = run_cmd("userdel", &["--remove", &job.unix_user]);
        return Err(e);
    }
    cgroup_written = true;

    // Step D: write PHP-FPM pool + reload FPM
    let result = write_php_fpm_pool(job)
        .and_then(|_| run_cmd("systemctl", &["reload", &format!("php{}-fpm", job.php_version)]));
    if let Err(e) = result {
        // Rollback C
        let slice = format!("website-{}.slice", job.domain);
        let _ = run_cmd("systemctl", &["stop", &slice]);
        let _ = std::fs::remove_file(format!("/etc/systemd/system/{}", slice));
        let _ = run_cmd("systemctl", &["daemon-reload"]);
        // Rollback A+B
        let _ = run_cmd("userdel", &["--remove", &job.unix_user]);
        return Err(e);
    }
    php_fpm_written = true;

    // Step E: write vhost + nginx reload
    let result = write_vhost(job)
        .and_then(|_| run_cmd("nginx", &["-t"]))
        .and_then(|_| run_cmd("systemctl", &["reload", "nginx"]));
    if let Err(e) = result {
        // Rollback D
        let pool_path = format!(
            "/etc/php/{}/fpm/pool.d/{}.conf",
            job.php_version, job.unix_user
        );
        let _ = std::fs::remove_file(&pool_path);
        let _ = run_cmd("systemctl", &["reload", &format!("php{}-fpm", job.php_version)]);
        // Rollback C
        let slice = format!("website-{}.slice", job.domain);
        let _ = run_cmd("systemctl", &["stop", &slice]);
        let _ = std::fs::remove_file(format!("/etc/systemd/system/{}", slice));
        let _ = run_cmd("systemctl", &["daemon-reload"]);
        // Rollback A+B
        let _ = run_cmd("userdel", &["--remove", &job.unix_user]);
        return Err(e);
    }
    vhost_written = true;

    // All provisioning steps succeeded.
    // The tracking vars are kept in scope in case a future step (e.g. quota) needs
    // to trigger a rollback; suppress unused-variable warnings explicitly.
    let _ = (unix_user_created, cgroup_written, php_fpm_written, vhost_written);

    // Apply disk quota if set in DB
    let quota_row = sqlx::query!(
        "SELECT disk_quota_mb FROM sites WHERE id = $1",
        job.site_id
    )
    .fetch_optional(&state.db)
    .await?;

    if let Some(row) = quota_row {
        if let Some(mb) = row.disk_quota_mb {
            if mb > 0 {
                if let Err(e) = crate::services::quota::set_site_quota(&job.unix_user, mb).await {
                    tracing::warn!(site_id = %job.site_id, error = %e, "Quota set failed (non-fatal)");
                }
            }
        }
    }

    Ok(())
}

// ── OS-level steps ────────────────────────────────────────────────────────────

fn create_unix_user(unix_user: &str) -> anyhow::Result<()> {
    // useradd is idempotent when user already exists (exit 9 = already exists; treat as OK)
    let status = std::process::Command::new("useradd")
        .args([
            "--system",
            "--no-create-home",
            "--shell", "/usr/sbin/nologin",
            "--comment", "JottiCP site",
            unix_user,
        ])
        .status()
        .map_err(|e| anyhow::anyhow!("useradd exec failed: {}", e))?;

    // exit 9 = user already exists — treat as success
    if !status.success() && status.code() != Some(9) {
        anyhow::bail!("useradd failed with status {:?} for user {}", status.code(), unix_user);
    }
    Ok(())
}

fn create_home_dir(unix_user: &str) -> anyhow::Result<()> {
    let home = format!("/home/{}/public_html", unix_user);
    std::fs::create_dir_all(&home)
        .map_err(|e| anyhow::anyhow!("mkdir -p {} failed: {}", home, e))?;

    run_cmd("chown", &["-R", &format!("{}:{}", unix_user, unix_user), &format!("/home/{}", unix_user)])?;
    run_cmd("chmod", &["750", &format!("/home/{}", unix_user)])?;
    Ok(())
}

fn write_cgroup_slice(domain: &str, unix_user: &str) -> anyhow::Result<()> {
    // Secondary path-traversal guard: domain must be strictly safe before embedding in a path.
    // (The caller already runs validate_domain, but belt-and-suspenders here.)
    anyhow::ensure!(
        !domain.contains('/') && !domain.contains(".."),
        "write_cgroup_slice: domain contains path traversal chars: {}",
        domain
    );

    // Escape the domain for use as a systemd slice unit name
    let slice_name = format!("website-{}.slice", domain);
    let slice_path = format!("/etc/systemd/system/{}", slice_name);

    // Use conservative limits for community tier; jotti-agent overrides for Pro
    let content = format!(
        "[Unit]\n\
         Description=JottiCP site cgroup slice for {domain}\n\
         DefaultDependencies=no\n\
         Before=slices.target\n\
         \n\
         [Slice]\n\
         # CPU: 20% max by default (Pro users can override via API)\n\
         CPUQuota=20%\n\
         # Memory: 256 MB soft limit; 512 MB hard (OOM killer)\n\
         MemoryHigh=256M\n\
         MemoryMax=512M\n\
         # Processes: prevent fork-bombs\n\
         TasksMax=100\n\
         # IO weight (relative scheduling)\n\
         IOWeight=100\n\
         # Owner\n\
         # unix_user: {unix_user}\n",
        domain   = domain,
        unix_user = unix_user,
    );

    std::fs::write(&slice_path, content)
        .map_err(|e| anyhow::anyhow!("write {} failed: {}", slice_path, e))?;

    Ok(())
}

fn write_php_fpm_pool(job: &ProvisioningJob) -> anyhow::Result<()> {
    let pool_name = &job.unix_user; // one pool per site, named after Unix user
    let socket_path = format!("/run/php/fpm-{}.sock", pool_name);

    let config = format!(
        "; JottiCP auto-generated PHP-FPM pool for {domain}\n\
         ; DO NOT EDIT — managed by jotti-panel. Changes will be overwritten.\n\
         [{pool}]\n\
         user  = {user}\n\
         group = {user}\n\
         \n\
         listen = {socket}\n\
         listen.owner = www-data\n\
         listen.group = www-data\n\
         ; 0666 so every front-end worker can connect to this socket — nginx/apache run as\n\
         ; www-data but OpenLiteSpeed runs as 'nobody'. PHP still executes as the site user\n\
         ; (pool user/group above) with open_basedir, so this is the connection endpoint only.\n\
         listen.mode  = 0666\n\
         \n\
         pm                   = ondemand\n\
         pm.max_children      = 10\n\
         pm.process_idle_timeout = 30s\n\
         pm.max_requests      = 500\n\
         \n\
         ; Restrict PHP to site homedir\n\
         php_admin_value[open_basedir] = /home/{user}/:/tmp/\n\
         php_admin_value[session.save_path] = /tmp/sessions/{user}\n\
         php_admin_value[upload_tmp_dir]    = /tmp/uploads/{user}\n\
         \n\
         ; OPcache (shared; tuned for per-site isolation)\n\
         php_admin_value[opcache.file_cache]           = /var/jottiecp/opcache/{user}\n\
         php_admin_flag[opcache.enable]                = on\n\
         php_admin_flag[opcache.enable_cli]            = off\n\
         php_admin_value[opcache.memory_consumption]   = 64\n\
         php_admin_value[opcache.max_accelerated_files] = 4000\n\
         php_admin_flag[opcache.jit]                   = on\n\
         php_admin_value[opcache.jit_buffer_size]      = 32M\n\
         \n\
         ; Error logging — never display to browser\n\
         php_admin_flag[display_errors] = off\n\
         php_admin_flag[log_errors]     = on\n\
         php_admin_value[error_log]     = /home/{user}/logs/php_error.log\n\
         \n\
         ; Disable dangerous functions\n\
         php_admin_value[disable_functions] = exec,passthru,shell_exec,system,proc_open,\
         popen,pcntl_exec\n\
         \n\
         ; Cgroup integration — attach pool to site slice\n\
         env[SYSTEMD_EXEC_PID] = $BASHPID\n",
        domain = job.domain,
        pool   = pool_name,
        user   = job.unix_user,
        socket = socket_path,
    );

    // Ensure per-user upload and session dirs exist
    for dir in &[
        format!("/tmp/sessions/{}", job.unix_user),
        format!("/tmp/uploads/{}", job.unix_user),
        format!("/var/jottiecp/opcache/{}", job.unix_user),
        format!("/home/{}/logs", job.unix_user),
    ] {
        std::fs::create_dir_all(dir)
            .map_err(|e| anyhow::anyhow!("mkdir {} failed: {}", dir, e))?;
    }

    let pool_path = format!(
        "/etc/php/{}/fpm/pool.d/{}.conf",
        job.php_version, pool_name
    );

    std::fs::write(&pool_path, config)
        .map_err(|e| anyhow::anyhow!("write PHP-FPM pool {} failed: {}", pool_path, e))?;

    Ok(())
}

fn write_vhost(job: &ProvisioningJob) -> anyhow::Result<()> {
    match job.web_server.as_str() {
        "nginx"  => write_nginx_vhost(job),
        "apache" => { write_nginx_proxy_vhost(&job.domain, 8081)?; write_apache_vhost(job) }
        "ols"    => { write_nginx_proxy_vhost(&job.domain, 8088)?; write_ols_vhost(job) }
        other    => anyhow::bail!("Unsupported web server: {}", other),
    }
}

/// Public wrapper so runtimes.rs can restore a PHP nginx vhost when switching back.
pub fn write_php_vhost_pub(job: &ProvisioningJob) -> anyhow::Result<()> {
    write_nginx_vhost(job)
}

fn write_nginx_vhost(job: &ProvisioningJob) -> anyhow::Result<()> {
    // Belt-and-suspenders path-traversal check: domain was already validated by regex
    // but we assert here so that the unsafe path can never be reached if validation is
    // accidentally skipped by a future caller.
    anyhow::ensure!(
        !job.domain.contains('/') && !job.domain.contains(".."),
        "write_nginx_vhost: domain contains path traversal chars: {}",
        job.domain
    );

    let socket_path = format!("/run/php/fpm-{}.sock", job.unix_user);

    let config = format!(
        "# JottiCP auto-generated vhost for {domain}\n\
         # DO NOT EDIT — managed by jotti-panel.\n\
         server {{\n\
             listen 80;\n\
             server_name {domain} www.{domain};\n\
             root /home/{user}/public_html;\n\
             index index.php index.html index.htm;\n\
         \n\
             access_log /home/{user}/logs/access.log combined;\n\
             error_log  /home/{user}/logs/error.log warn;\n\
         \n\
             # Max upload size — override in per-site config\n\
             client_max_body_size 50M;\n\
         \n\
             location / {{\n\
                 try_files $uri $uri/ /index.php?$query_string;\n\
             }}\n\
         \n\
             location ~ \\.php$ {{\n\
                 include        snippets/fastcgi-php.conf;\n\
                 fastcgi_pass   unix:{socket};\n\
                 fastcgi_param  SCRIPT_FILENAME $document_root$fastcgi_script_name;\n\
                 include        fastcgi_params;\n\
                 # Prevent PHP from processing files outside webroot\n\
                 fastcgi_param  PHP_VALUE \"open_basedir=/home/{user}/:/tmp/\";\n\
             }}\n\
         \n\
             # Deny access to hidden files (.htaccess, .env, .git, etc.)\n\
             location ~ /\\. {{\n\
                 deny all;\n\
                 return 404;\n\
             }}\n\
         \n\
             # ACME HTTP-01 challenge dir\n\
             location ^~ /.well-known/acme-challenge/ {{\n\
                 root /var/lib/jottiecp/acme/{domain};\n\
                 try_files $uri =404;\n\
             }}\n\
         }}\n",
        domain = job.domain,
        user   = job.unix_user,
        socket = socket_path,
    );

    let vhost_path = format!("/etc/nginx/sites-available/{}.conf", job.domain);
    std::fs::write(&vhost_path, &config)
        .map_err(|e| anyhow::anyhow!("write nginx vhost {} failed: {}", vhost_path, e))?;

    // Enable the site
    let enabled_path = format!("/etc/nginx/sites-enabled/{}.conf", job.domain);
    if !std::path::Path::new(&enabled_path).exists() {
        std::os::unix::fs::symlink(&vhost_path, &enabled_path)
            .map_err(|e| anyhow::anyhow!("symlink nginx vhost failed: {}", e))?;
    }

    // nginx serves the docroot DIRECTLY as www-data, but the site home is 0750 user:user,
    // so the worker cannot traverse it → 404. Grant www-data an ACL (traverse on home,
    // read+traverse on docroot), mirroring the apache path. Without this, fresh nginx sites
    // 404 until some other op (e.g. an apache switch) happens to add the ACL.
    let home    = format!("/home/{}", job.unix_user);
    let docroot = format!("/home/{}/public_html", job.unix_user);
    let _ = run_cmd("setfacl", &["-m", "u:www-data:--x", &home]);
    let _ = run_cmd("setfacl", &["-R", "-m", "u:www-data:rX", &docroot]);
    let _ = run_cmd("setfacl", &["-d", "-m", "u:www-data:rX", &docroot]);

    Ok(())
}

/// Write an nginx vhost that proxies all traffic to a local backend (apache:8081 or ols:8088).
fn write_nginx_proxy_vhost(domain: &str, backend_port: u16) -> anyhow::Result<()> {
    anyhow::ensure!(
        !domain.contains('/') && !domain.contains(".."),
        "write_nginx_proxy_vhost: domain contains path traversal chars: {}", domain
    );

    let config = format!(
        "# JottiCP auto-generated nginx proxy vhost for {domain} → :{port}\n\
         # DO NOT EDIT — managed by jotti-panel.\n\
         server {{\n\
             listen 80;\n\
             server_name {domain} www.{domain};\n\
         \n\
             access_log /var/log/nginx/{domain}-access.log combined;\n\
             error_log  /var/log/nginx/{domain}-error.log warn;\n\
         \n\
             # ACME HTTP-01 challenge\n\
             location ^~ /.well-known/acme-challenge/ {{\n\
                 root /var/lib/jottiecp/acme/{domain};\n\
                 try_files $uri =404;\n\
             }}\n\
         \n\
             location / {{\n\
                 proxy_pass         http://127.0.0.1:{port};\n\
                 proxy_set_header   Host $host;\n\
                 proxy_set_header   X-Real-IP $remote_addr;\n\
                 proxy_set_header   X-Forwarded-For $proxy_add_x_forwarded_for;\n\
                 proxy_set_header   X-Forwarded-Proto $scheme;\n\
                 proxy_read_timeout 120s;\n\
                 proxy_connect_timeout 10s;\n\
             }}\n\
         }}\n",
        domain = domain,
        port   = backend_port,
    );

    let vhost_path = format!("/etc/nginx/sites-available/{}.conf", domain);
    std::fs::write(&vhost_path, &config)
        .map_err(|e| anyhow::anyhow!("write nginx proxy vhost {} failed: {}", vhost_path, e))?;

    let enabled_path = format!("/etc/nginx/sites-enabled/{}.conf", domain);
    if !std::path::Path::new(&enabled_path).exists() {
        std::os::unix::fs::symlink(&vhost_path, &enabled_path)
            .map_err(|e| anyhow::anyhow!("symlink nginx proxy vhost failed: {}", e))?;
    }

    Ok(())
}

fn write_apache_vhost(job: &ProvisioningJob) -> anyhow::Result<()> {
    anyhow::ensure!(
        !job.domain.contains('/') && !job.domain.contains(".."),
        "write_apache_vhost: domain contains path traversal chars: {}",
        job.domain
    );

    let socket_path = format!("/run/php/fpm-{}.sock", job.unix_user);

    // Apache listens on 8081 (nginx proxies 80/443 → 8081)
    let config = format!(
        "# JottiCP auto-generated Apache vhost for {domain}\n\
         <VirtualHost *:8081>\n\
             ServerName  {domain}\n\
             ServerAlias www.{domain}\n\
             DocumentRoot /home/{user}/public_html\n\
         \n\
             ErrorLog  /home/{user}/logs/error.log\n\
             CustomLog /home/{user}/logs/access.log combined\n\
         \n\
             <Directory /home/{user}/public_html>\n\
                 Options FollowSymLinks\n\
                 AllowOverride All\n\
                 Require all granted\n\
             </Directory>\n\
         \n\
             <FilesMatch \\.php$>\n\
                 SetHandler  \"proxy:unix:{socket}|fcgi://localhost\"\n\
             </FilesMatch>\n\
         \n\
             <FilesMatch \"^\\.\">\n\
                 Require all denied\n\
             </FilesMatch>\n\
         </VirtualHost>\n",
        domain = job.domain,
        user   = job.unix_user,
        socket = socket_path,
    );

    let vhost_path = format!("/etc/apache2/sites-available/{}.conf", job.domain);
    std::fs::write(&vhost_path, &config)
        .map_err(|e| anyhow::anyhow!("write apache vhost {} failed: {}", vhost_path, e))?;

    // Apache runs as www-data; the site home is 0750 user:user, so www-data cannot
    // traverse it → 403. Grant www-data an ACL (traverse on home, read+traverse on docroot)
    // so Apache can serve the files. Non-fatal if ACLs are unavailable.
    let home = format!("/home/{}", job.unix_user);
    let docroot = format!("/home/{}/public_html", job.unix_user);
    let _ = run_cmd("setfacl", &["-m", "u:www-data:--x", &home]);
    let _ = run_cmd("setfacl", &["-R", "-m", "u:www-data:rX", &docroot]);
    let _ = run_cmd("setfacl", &["-d", "-m", "u:www-data:rX", &docroot]);

    run_cmd("a2ensite", &[&format!("{}.conf", job.domain)])?;
    run_cmd("apachectl", &["configtest"])?;
    run_cmd("systemctl", &["reload", "apache2"])?;

    Ok(())
}

fn write_ols_vhost(job: &ProvisioningJob) -> anyhow::Result<()> {
    anyhow::ensure!(
        !job.domain.contains('/') && !job.domain.contains(".."),
        "write_ols_vhost: domain contains path traversal chars: {}",
        job.domain
    );

    let vhost_dir = format!("/usr/local/lsws/conf/vhosts/{}", job.domain);
    std::fs::create_dir_all(&vhost_dir)
        .map_err(|e| anyhow::anyhow!("create OLS vhost dir {} failed: {}", vhost_dir, e))?;

    // OLS vhost config — uses existing PHP-FPM socket via fastcgi extprocessor
    let vhconf = format!(
        "docRoot                   /home/{user}/public_html\n\
         vhDomain                  {domain}\n\
         vhAliases                 www.{domain}\n\
         enableGzip                1\n\
         \n\
         index  {{\n\
           useServer               0\n\
           indexFiles              index.php, index.html, index.htm\n\
           autoIndex               0\n\
         }}\n\
         \n\
         scripthandler  {{\n\
           add                     fcgi:fpm-{user} php\n\
         }}\n\
         \n\
         extprocessor fpm-{user}  {{\n\
           type                    fcgi\n\
           address                 uds:///run/php/fpm-{user}.sock\n\
           maxConns                100\n\
           initTimeout             60\n\
           retryTimeout            0\n\
           pcKeepAliveTimeout      300\n\
           respBuffer              0\n\
           autoStart               0\n\
         }}\n\
         \n\
         accesslog  {{\n\
           location                /home/{user}/logs/access.log\n\
           logFormat               combined\n\
           rollingSize             10M\n\
           keepDays                10\n\
         }}\n\
         \n\
         errorlog  {{\n\
           location                /home/{user}/logs/error.log\n\
           logLevel                WARN\n\
           rollingSize             10M\n\
         }}\n",
        domain = job.domain,
        user   = job.unix_user,
    );

    let vhconf_path = format!("{}/vhconf.conf", vhost_dir);
    std::fs::write(&vhconf_path, &vhconf)
        .map_err(|e| anyhow::anyhow!("write OLS vhconf {} failed: {}", vhconf_path, e))?;

    // OpenLiteSpeed serves as 'nobody', but the site home is 0750 user:user → nobody cannot
    // traverse it to reach the docroot → 404. Grant nobody an ACL (traverse on home,
    // read+traverse on docroot), mirroring the www-data ACL on the nginx/apache paths.
    let home    = format!("/home/{}", job.unix_user);
    let docroot = format!("/home/{}/public_html", job.unix_user);
    let _ = run_cmd("setfacl", &["-m", "u:nobody:--x", &home]);
    let _ = run_cmd("setfacl", &["-R", "-m", "u:nobody:rX", &docroot]);
    let _ = run_cmd("setfacl", &["-d", "-m", "u:nobody:rX", &docroot]);

    // Register the vhost in httpd_config.conf: a top-level `virtualhost` block AND a
    // per-domain `map` inside the listener. WITHOUT the listener map, OLS routes every
    // request to the catch-all vhost (`map Example *`) → the new domain 404s.
    let httpd_conf = "/usr/local/lsws/conf/httpd_config.conf";
    if std::path::Path::new(httpd_conf).exists() {
        let mut content = std::fs::read_to_string(httpd_conf).unwrap_or_default();
        let mut changed = false;

        // 1. virtualhost block (top-level) if absent
        let vh_marker = format!("virtualhost {} {{", job.domain);
        let vh_marker2 = format!("vhRoot                  /usr/local/lsws/conf/vhosts/{}/", job.domain);
        if !content.contains(&vh_marker) && !content.contains(&vh_marker2) {
            content.push_str(&format!(
                "\nvirtualhost {domain} {{\n  \
                   vhRoot                  /usr/local/lsws/conf/vhosts/{domain}/\n  \
                   configFile              $VH_ROOT/vhconf.conf\n  \
                   allowSymbolLink         1\n  \
                   enableScript            1\n  \
                   restrained              1\n\
                 }}\n",
                domain = job.domain));
            changed = true;
        }

        // 2. per-domain listener map if absent. Insert before the closing brace of the
        // FIRST `listener <name>{` block (works regardless of listener name, e.g. "Default").
        let map_present = content.lines().any(|l| {
            let t = l.trim();
            t.starts_with("map ") && t.contains(&format!(" {} ", job.domain))
        });
        if !map_present {
            if let Some(lstart) = content.find("\nlistener ") {
                if let Some(rel) = content[lstart..].find("\n}") {
                    let insert_at = lstart + rel; // before the listener's "\n}"
                    let map_line = format!(
                        "\n    map                      {dom} {dom}, www.{dom}",
                        dom = job.domain);
                    content.insert_str(insert_at, &map_line);
                    changed = true;
                }
            }
        }

        if changed {
            std::fs::write(httpd_conf, &content)
                .map_err(|e| anyhow::anyhow!("update OLS httpd_config.conf failed: {}", e))?;
        }
    }

    // Restart OLS to pick up the new vhost + listener map. OLS's graceful restart can fail
    // to rebind the admin listener (:7080) if the previous instance hasn't released it yet
    // ("failed to start listener on address *:7080 ... Fatal error, exit") — leaving the OLD
    // config running. A short pause + second restart reliably applies the new config; the
    // first (possibly failed) restart leaves the existing server serving in the meantime.
    let _ = run_cmd("/usr/local/lsws/bin/lswsctrl", &["restart"]);
    std::thread::sleep(std::time::Duration::from_secs(4));
    let _ = run_cmd("/usr/local/lsws/bin/lswsctrl", &["restart"]);

    Ok(())
}

// ── Deprovision ───────────────────────────────────────────────────────────────

pub async fn deprovision_site(
    state: &Arc<AppState>,
    site_id: Uuid,
    domain: &str,
    unix_user: &str,
) -> anyhow::Result<()> {
    validate_domain(domain)?;
    validate_unix_user(unix_user)?;

    tracing::info!(site_id = %site_id, domain = %domain, "Starting site deprovisioning");

    // 1. Stop cgroup slice (graceful)
    let slice = format!("website-{}.slice", domain);
    let _ = run_cmd("systemctl", &["stop", &slice]);

    // 2. Remove PHP-FPM pool config + reload
    let pool_paths: Vec<String> = ["8.1", "8.2", "8.3", "8.4"]
        .iter()
        .map(|v| format!("/etc/php/{}/fpm/pool.d/{}.conf", v, unix_user))
        .collect();

    for path in &pool_paths {
        if std::path::Path::new(path).exists() {
            let _ = std::fs::remove_file(path);
            // Find the PHP version from the path and reload that FPM
            let ver = path.split('/').nth(3).unwrap_or("8.3");
            let _ = run_cmd("systemctl", &["reload", &format!("php{}-fpm", ver)]);
        }
    }

    // 3. Remove nginx vhost
    let available = format!("/etc/nginx/sites-available/{}.conf", domain);
    let enabled   = format!("/etc/nginx/sites-enabled/{}.conf", domain);
    let _ = std::fs::remove_file(&enabled);
    let _ = std::fs::remove_file(&available);
    let _ = run_cmd("nginx", &["-t"]);
    let _ = run_cmd("systemctl", &["reload", "nginx"]);

    // 4. Remove apache vhost if present
    let apache_conf = format!("/etc/apache2/sites-available/{}.conf", domain);
    if std::path::Path::new(&apache_conf).exists() {
        let _ = run_cmd("a2dissite", &[&format!("{}.conf", domain)]);
        let _ = std::fs::remove_file(&apache_conf);
        let _ = run_cmd("systemctl", &["reload", "apache2"]);
    }

    // 5. Delete Unix user + homedir
    let _ = run_cmd("userdel", &["--remove", unix_user]);

    // 6. Remove cgroup slice unit file
    let slice_path = format!("/etc/systemd/system/{}", slice);
    let _ = std::fs::remove_file(&slice_path);
    let _ = run_cmd("systemctl", &["daemon-reload"]);

    // 7. Remove ACME challenge dir
    let _ = std::fs::remove_dir_all(format!("/var/lib/jottiecp/acme/{}", domain));

    // 8. Mark deleted in DB
    sqlx::query!(
        "UPDATE sites SET status = 'deleted', deleted_at = NOW() WHERE id = $1",
        site_id
    )
    .execute(&state.db)
    .await?;

    write_audit_log(
        state,
        "site_deprovisioned",
        &site_id.to_string(),
        &format!("domain={} user={}", domain, unix_user),
    )
    .await;

    tracing::info!(site_id = %site_id, "Site deprovisioned");
    Ok(())
}

// ── Suspend / unsuspend ───────────────────────────────────────────────────────

pub async fn suspend_site_files(site_id: Uuid, domain: &str) -> anyhow::Result<()> {
    validate_domain(domain)?;

    // Write a temporary "suspended" nginx vhost overlay
    let suspended_vhost = format!(
        "# JottiCP — site SUSPENDED\n\
         server {{\n\
             listen 80;\n\
             server_name {domain} www.{domain};\n\
             return 403 \"This site has been suspended. Contact support.\";\n\
         }}\n",
        domain = domain,
    );

    let vhost_path = format!("/etc/nginx/sites-available/{}.conf", domain);
    std::fs::write(&vhost_path, suspended_vhost)
        .map_err(|e| anyhow::anyhow!("write suspended vhost failed: {}", e))?;

    run_cmd("nginx", &["-t"])?;
    run_cmd("systemctl", &["reload", "nginx"])?;

    tracing::info!(site_id = %site_id, domain = %domain, "Site suspended");
    Ok(())
}

pub async fn unsuspend_site_files(state: &Arc<AppState>, site_id: Uuid) -> anyhow::Result<()> {
    let site = sqlx::query!(
        "SELECT domain, unix_user, php_version, web_server FROM sites WHERE id = $1",
        site_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| anyhow::anyhow!("Site {} not found for unsuspend", site_id))?;

    validate_domain(&site.domain)?;
    validate_unix_user(&site.unix_user)?;

    let job = ProvisioningJob {
        site_id,
        domain:      site.domain,
        php_version: site.php_version,
        web_server:  site.web_server,
        unix_user:   site.unix_user,
        server_id:   None,
    };

    // Restore the real vhost
    write_vhost(&job)?;
    run_cmd("nginx", &["-t"])?;
    run_cmd("systemctl", &["reload", "nginx"])?;

    tracing::info!(site_id = %site_id, "Site unsuspended");
    Ok(())
}

// ── PHP version change ────────────────────────────────────────────────────────

pub async fn change_php_version(
    state: &Arc<AppState>,
    site_id: Uuid,
    new_version: &str,
) -> anyhow::Result<()> {
    validate_php_version(new_version)?;

    let site = sqlx::query!(
        "SELECT domain, unix_user, php_version, web_server FROM sites WHERE id = $1",
        site_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| anyhow::anyhow!("Site {} not found", site_id))?;

    validate_domain(&site.domain)?;
    validate_unix_user(&site.unix_user)?;

    // Remove this site's FPM pool from EVERY installed PHP version (not just the
    // DB-recorded one). Leaving stale pools in other version dirs makes multiple
    // php{ver}-fpm masters try to bind the same uds socket → reload/start failures.
    for ver in ["7.4", "8.0", "8.1", "8.2", "8.3", "8.4"] {
        let pool = format!("/etc/php/{}/fpm/pool.d/{}.conf", ver, site.unix_user);
        if std::path::Path::new(&pool).exists() {
            let _ = std::fs::remove_file(&pool);
            let _ = run_cmd("systemctl", &["reload", &format!("php{}-fpm", ver)]);
        }
    }

    // Write new pool config
    let job = ProvisioningJob {
        site_id,
        domain:      site.domain,
        php_version: new_version.to_string(),
        web_server:  site.web_server,
        unix_user:   site.unix_user,
        server_id:   None,
    };
    write_php_fpm_pool(&job)?;
    run_cmd("systemctl", &["reload", &format!("php{}-fpm", new_version)])?;

    sqlx::query!(
        "UPDATE sites SET php_version = $1, updated_at = NOW() WHERE id = $2",
        new_version, site_id
    )
    .execute(&state.db)
    .await?;

    tracing::info!(site_id = %site_id, version = %new_version, "PHP version changed");
    Ok(())
}

// ── Webserver switch ──────────────────────────────────────────────────────────

/// Switch the webserver for an existing site. Sync — call via spawn_blocking.
/// Removes the current nginx vhost (and any backend vhost), writes the new one,
/// and reloads the affected services.
pub fn switch_webserver_sync(domain: &str, unix_user: &str, new_ws: &str, php_version: &str) -> anyhow::Result<()> {
    // Remove existing nginx vhost (both available + enabled symlink)
    let available = format!("/etc/nginx/sites-available/{}.conf", domain);
    let enabled   = format!("/etc/nginx/sites-enabled/{}.conf", domain);
    let _ = std::fs::remove_file(&enabled);
    let _ = std::fs::remove_file(&available);

    // Remove stale apache vhost if switching away from apache
    let apache_conf = format!("/etc/apache2/sites-available/{}.conf", domain);
    if std::path::Path::new(&apache_conf).exists() {
        let _ = run_cmd("a2dissite", &[&format!("{}.conf", domain)]);
        let _ = std::fs::remove_file(&apache_conf);
    }

    // Build a minimal ProvisioningJob for the vhost writers
    let ver = if php_version.is_empty() { "8.3" } else { php_version };
    let job = ProvisioningJob {
        site_id:     uuid::Uuid::nil(),
        domain:      domain.to_string(),
        php_version: ver.to_string(),
        web_server:  new_ws.to_string(),
        unix_user:   unix_user.to_string(),
        server_id:   None,
    };

    // Ensure the site's PHP-FPM pool exists for the current version so the new
    // vhost (which points at /run/php/fpm-<user>.sock) can actually execute PHP.
    let _ = write_php_fpm_pool(&job);
    let _ = run_cmd("systemctl", &["reload", &format!("php{}-fpm", ver)]);

    match new_ws {
        "nginx" => {
            write_nginx_vhost(&job)?;
        }
        "apache" => {
            write_nginx_proxy_vhost(domain, 8081)?;
            write_apache_vhost(&job)?;
        }
        "ols" => {
            write_nginx_proxy_vhost(domain, 8088)?;
            write_ols_vhost(&job)?;
        }
        other => anyhow::bail!("Unsupported web server: {}", other),
    }

    // Validate + reload nginx
    run_cmd("nginx", &["-t"])?;
    run_cmd("systemctl", &["reload", "nginx"])?;

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Run an external command, capturing stderr on failure.
fn run_cmd(program: &str, args: &[&str]) -> anyhow::Result<()> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|e| anyhow::anyhow!("exec '{}' failed: {}", program, e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "Command '{} {}' exited {:?}: {}",
            program,
            args.join(" "),
            output.status.code(),
            stderr.trim()
        );
    }
    Ok(())
}

// ── Runtime provisioning (Steps 4.9–4.12) ────────────────────────────────────

/// Dispatch a language runtime setup to the appropriate jotti-agent via gRPC
/// (or locally if no server_id is given).
///
/// `runtime` must be one of "python" | "nodejs" | "ruby" | "dotnet".
/// The actual work is done inside jotti-agent/src/handlers/runtime.rs.
// ── Runtime provisioners (direct, no agent) ───────────────────────────────────

/// Tear down any previously active runtime nginx vhost and app process for a
/// domain, so a new runtime can be written cleanly.
pub fn deprovision_runtime_sync(domain: &str, unix_user: &str, prev_runtime: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !domain.contains('/') && !domain.contains(".."),
        "deprovision_runtime: domain unsafe: {}", domain
    );
    anyhow::ensure!(
        !unix_user.contains('/') && !unix_user.contains(".."),
        "deprovision_runtime: unix_user unsafe: {}", unix_user
    );

    // Stop runtime-specific process (both nodejs and python use systemd units)
    match prev_runtime {
        "nodejs" => {
            let unit = format!("jottiecp-nodejs-{}", domain);
            // Capture MainPID before stopping so we can force-kill orphans
            let main_pid: Option<u32> = std::process::Command::new("systemctl")
                .args(["show", &unit, "--property=MainPID", "--value"])
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .and_then(|s| s.trim().parse::<u32>().ok())
                .filter(|&p| p > 0);
            let _ = run_cmd("systemctl", &["stop",    &unit]);
            let _ = run_cmd("systemctl", &["disable", &unit]);
            let unit_path = format!("/etc/systemd/system/{}.service", unit);
            let _ = std::fs::remove_file(&unit_path);
            let _ = run_cmd("systemctl", &["daemon-reload"]);
            // Force-kill if process outlived the unit (e.g. systemctl stop returned before exit)
            if let Some(pid) = main_pid {
                if std::path::Path::new(&format!("/proc/{}", pid)).exists() {
                    let _ = run_cmd("kill", &["-9", &pid.to_string()]);
                }
            }
        }
        "python" => {
            let unit = format!("jottiecp-python-{}", domain);
            let main_pid: Option<u32> = std::process::Command::new("systemctl")
                .args(["show", &unit, "--property=MainPID", "--value"])
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .and_then(|s| s.trim().parse::<u32>().ok())
                .filter(|&p| p > 0);
            let _ = run_cmd("systemctl", &["stop",    &unit]);
            let _ = run_cmd("systemctl", &["disable", &unit]);
            let unit_path = format!("/etc/systemd/system/{}.service", unit);
            let _ = std::fs::remove_file(&unit_path);
            let _ = run_cmd("systemctl", &["daemon-reload"]);
            if let Some(pid) = main_pid {
                if std::path::Path::new(&format!("/proc/{}", pid)).exists() {
                    let _ = run_cmd("kill", &["-9", &pid.to_string()]);
                }
            }
        }
        _ => {}
    }

    // Remove nginx vhost for this domain (proxy or direct — both share same filename)
    let available = format!("/etc/nginx/sites-available/{}.conf", domain);
    let enabled   = format!("/etc/nginx/sites-enabled/{}.conf",   domain);
    let _ = std::fs::remove_file(&enabled);
    let _ = std::fs::remove_file(&available);

    Ok(())
}

/// Provision a Node.js runtime for a site via systemd (consistent with Python approach,
/// no CAP_SETUID required since systemd handles user switching).
///
/// 1. Write nginx proxy vhost → port
/// 2. Reload nginx
/// 3. Write default server.js placeholder if no app files exist
/// 4. Write + enable systemd unit running as unix_user
pub fn provision_nodejs_sync(domain: &str, unix_user: &str, port: i32) -> anyhow::Result<()> {
    anyhow::ensure!(!domain.contains('/') && !domain.contains(".."), "domain unsafe");
    anyhow::ensure!(!unix_user.contains('/') && !unix_user.contains(".."), "unix_user unsafe");
    anyhow::ensure!((3001..=4999).contains(&port), "port {} out of range", port);
    anyhow::ensure!(
        std::process::Command::new("id").arg(unix_user).status().map(|s| s.success()).unwrap_or(false),
        "Unix user '{}' does not exist on this system — re-provision the site first.", unix_user
    );

    let docroot   = format!("/home/{}/public_html", unix_user);
    let log_dir   = format!("/var/log/jottiecp/sites/{}", domain);
    let unit_name = format!("jottiecp-nodejs-{}", domain);
    let unit_path = format!("/etc/systemd/system/{}.service", unit_name);

    std::fs::create_dir_all(&log_dir).unwrap_or(());

    // 1. Write nginx proxy vhost
    write_runtime_proxy_vhost(domain, port as u16)?;
    run_cmd("nginx", &["-t"])?;
    run_cmd("systemctl", &["reload", "nginx"])?;

    // 2. Write placeholder server.js only if no app files exist yet
    let server_js = format!("{}/server.js", docroot);
    if !std::path::Path::new(&server_js).exists()
        && !std::path::Path::new(&format!("{}/package.json", docroot)).exists()
    {
        let placeholder = format!(
            "// JottiCP — placeholder server for {domain}\n\
             // Replace this file with your real Node.js application.\n\
             const http = require('http');\n\
             const PORT = process.env.PORT || {port};\n\
             http.createServer((req, res) => {{\n\
               res.writeHead(200, {{'Content-Type': 'text/plain'}});\n\
               res.end('JottiCP: Deploy your app to /home/{unix_user}/public_html/\\n');\n\
             }}).listen(PORT, '127.0.0.1', () => console.log('Listening on ' + PORT));\n",
            domain    = domain,
            port      = port,
            unix_user = unix_user,
        );
        std::fs::write(&server_js, placeholder)
            .map_err(|e| anyhow::anyhow!("write placeholder server.js failed: {}", e))?;
        let _ = run_cmd("chown", &[&format!("{}:{}", unix_user, unix_user), &server_js]);
    }

    // 3. Write systemd unit — systemd handles user switch (no CAP_SETUID needed)
    let unit_content = format!(
        "[Unit]\n\
         Description=JottiCP Node.js app: {domain}\n\
         After=network.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         User={unix_user}\n\
         WorkingDirectory={docroot}\n\
         Environment=\"PORT={port}\"\n\
         Environment=\"NODE_ENV=production\"\n\
         ExecStart=/usr/bin/node server.js\n\
         Restart=always\n\
         RestartSec=5\n\
         StandardOutput=append:{log_dir}/nodejs.log\n\
         StandardError=append:{log_dir}/nodejs-error.log\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        domain    = domain,
        unix_user = unix_user,
        docroot   = docroot,
        port      = port,
        log_dir   = log_dir,
    );
    std::fs::write(&unit_path, unit_content)
        .map_err(|e| anyhow::anyhow!("write systemd unit {} failed: {}", unit_path, e))?;

    run_cmd("systemctl", &["daemon-reload"])?;
    run_cmd("systemctl", &["enable", "--now", &unit_name])?;

    Ok(())
}

/// Provision a Python runtime for a site:
/// 1. Create per-site venv (idempotent)
/// 2. Install gunicorn + uvicorn into the venv
/// 3. Write nginx proxy vhost → port
/// 4. Reload nginx
/// 5. Write + enable a systemd unit for gunicorn/uvicorn
pub fn provision_python_sync(
    domain:      &str,
    unix_user:   &str,
    port:        i32,
    entry_point: &str,  // e.g. "main:app", "wsgi:application"
) -> anyhow::Result<()> {
    anyhow::ensure!(!domain.contains('/') && !domain.contains(".."), "domain unsafe");
    anyhow::ensure!(!unix_user.contains('/') && !unix_user.contains(".."), "unix_user unsafe");
    anyhow::ensure!((3001..=4999).contains(&port), "port {} out of range", port);
    anyhow::ensure!(
        std::process::Command::new("id").arg(unix_user).status().map(|s| s.success()).unwrap_or(false),
        "Unix user '{}' does not exist on this system — re-provision the site first.", unix_user
    );

    // Strictly validate entry_point: must be module.submod:callable form only.
    // Banning spaces prevents multi-argument injection into systemd ExecStart.
    let ep = if entry_point.is_empty() { "main:app" } else { entry_point };
    {
        let re = regex::Regex::new(r"^[a-zA-Z_][a-zA-Z0-9_.]*:[a-zA-Z_][a-zA-Z0-9_]*$").unwrap();
        anyhow::ensure!(re.is_match(ep),
            "entry_point must be 'module:callable' form (e.g. 'main:app'); got: {}", ep);
    }

    let venv_path   = format!("/home/{}/venv",        unix_user);
    let docroot     = format!("/home/{}/public_html",  unix_user);
    let log_dir     = format!("/var/log/jottiecp/sites/{}", domain);
    let unit_name   = format!("jottiecp-python-{}", domain);
    let unit_path   = format!("/etc/systemd/system/{}.service", unit_name);

    std::fs::create_dir_all(&log_dir).unwrap_or(());

    // 1. Create venv (idempotent — skips if already exists)
    if !std::path::Path::new(&format!("{}/bin/python3", venv_path)).exists() {
        run_cmd("python3", &["-m", "venv", &venv_path])?;
        run_cmd("chown", &["-R", &format!("{}:{}", unix_user, unix_user), &venv_path])?;
    }

    // 2. Install gunicorn + uvicorn in the venv (idempotent).
    // pip runs as root here; chown after install restores correct ownership.
    let pip = format!("{}/bin/pip", venv_path);
    run_cmd(&pip, &["install", "--quiet", "--upgrade", "gunicorn", "uvicorn[standard]"])?;
    run_cmd("chown", &["-R", &format!("{}:{}", unix_user, unix_user), &venv_path])?;

    // 3. Write nginx proxy vhost
    write_runtime_proxy_vhost(domain, port as u16)?;
    run_cmd("nginx", &["-t"])?;
    run_cmd("systemctl", &["reload", "nginx"])?;

    // 4. Decide: uvicorn (ASGI) if entry contains 'asgi' or 'fastapi' pattern,
    //    otherwise gunicorn. User can override with custom entry_point.
    let use_uvicorn = ep.contains("asgi") || ep.contains("fastapi");
    let server_bin  = format!("{}/bin/{}", venv_path, if use_uvicorn { "uvicorn" } else { "gunicorn" });

    let exec_start = if use_uvicorn {
        format!("{} {} --host 127.0.0.1 --port {} --workers 2", server_bin, ep, port)
    } else {
        format!("{} {} --bind 127.0.0.1:{} --workers 2 --timeout 120", server_bin, ep, port)
    };

    // 5. Write systemd unit
    let unit_content = format!(
        "[Unit]\n\
         Description=JottiCP Python app: {domain}\n\
         After=network.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         User={unix_user}\n\
         WorkingDirectory={docroot}\n\
         Environment=\"PORT={port}\"\n\
         Environment=\"PYTHONPATH={docroot}\"\n\
         ExecStart={exec_start}\n\
         Restart=always\n\
         RestartSec=5\n\
         StandardOutput=append:{log_dir}/python.log\n\
         StandardError=append:{log_dir}/python-error.log\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        domain       = domain,
        unix_user    = unix_user,
        docroot      = docroot,
        port         = port,
        exec_start   = exec_start,
        log_dir      = log_dir,
    );
    std::fs::write(&unit_path, unit_content)
        .map_err(|e| anyhow::anyhow!("write systemd unit {} failed: {}", unit_path, e))?;

    run_cmd("systemctl", &["daemon-reload"])?;
    run_cmd("systemctl", &["enable", "--now", &unit_name])?;

    Ok(())
}

/// Write an nginx proxy vhost config with WebSocket upgrade support,
/// suitable for both Node.js and Python apps.
fn write_runtime_proxy_vhost(domain: &str, port: u16) -> anyhow::Result<()> {
    anyhow::ensure!(!domain.contains('/') && !domain.contains(".."), "domain unsafe");

    let config = format!(
        "# JottiCP runtime proxy vhost: {domain} → 127.0.0.1:{port}\n\
         # DO NOT EDIT — managed by jotti-panel.\n\
         server {{\n\
             listen 80;\n\
             server_name {domain} www.{domain};\n\
         \n\
             access_log /var/log/nginx/{domain}-access.log combined;\n\
             error_log  /var/log/nginx/{domain}-error.log warn;\n\
         \n\
             client_max_body_size 100M;\n\
         \n\
             location ^~ /.well-known/acme-challenge/ {{\n\
                 root /var/lib/jottiecp/acme/{domain};\n\
                 try_files $uri =404;\n\
             }}\n\
         \n\
             location / {{\n\
                 proxy_pass         http://127.0.0.1:{port};\n\
                 proxy_http_version 1.1;\n\
                 proxy_set_header   Upgrade           $http_upgrade;\n\
                 proxy_set_header   Connection        \"upgrade\";\n\
                 proxy_set_header   Host              $host;\n\
                 proxy_set_header   X-Real-IP         $remote_addr;\n\
                 proxy_set_header   X-Forwarded-For   $proxy_add_x_forwarded_for;\n\
                 proxy_set_header   X-Forwarded-Proto $scheme;\n\
                 proxy_read_timeout    120s;\n\
                 proxy_connect_timeout  10s;\n\
             }}\n\
         }}\n",
        domain = domain,
        port   = port,
    );

    let available = format!("/etc/nginx/sites-available/{}.conf", domain);
    let enabled   = format!("/etc/nginx/sites-enabled/{}.conf",   domain);

    std::fs::write(&available, &config)
        .map_err(|e| anyhow::anyhow!("write runtime proxy vhost failed: {}", e))?;

    if !std::path::Path::new(&enabled).exists() {
        std::os::unix::fs::symlink(&available, &enabled)
            .map_err(|e| anyhow::anyhow!("symlink runtime proxy vhost failed: {}", e))?;
    }

    Ok(())
}

// run_cmd_as and get_uid_gid removed: jotti-panel lacks CAP_SETUID (NoNewPrivileges=true
// service), so uid/gid drops fail with EPERM. All app process management now uses
// systemd units (which handles user switching internally at daemon level).

pub async fn setup_runtime_site(
    state: &Arc<AppState>,
    site_id: Uuid,
    domain: &str,
    unix_user: &str,
    runtime: &str,
    version: &str,
    server_id: Option<Uuid>,
) -> anyhow::Result<()> {
    validate_domain(domain)?;
    validate_unix_user(unix_user)?;

    tracing::info!(
        site_id = %site_id,
        %domain,
        %runtime,
        %version,
        "Provisioning language runtime"
    );

    // If a remote server is specified, call jotti-agent via HTTP (gRPC stub).
    // Full tonic client wiring is completed in Step 4.3; this sends a JSON body
    // to the agent's /agent/<runtime> endpoint as a compatibility shim.
    if let Some(sid) = server_id {
        let server = sqlx::query!(
            "SELECT agent_address FROM servers WHERE id = $1 AND deleted_at IS NULL",
            sid
        )
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Server {} not found", sid))?;

        let agent_addr = server.agent_address
            .ok_or_else(|| anyhow::anyhow!("Server {} has no agent address", sid))?;

        let body = serde_json::json!({
            "site_id":   site_id,
            "domain":    domain,
            "unix_user": unix_user,
            "runtime":   runtime,
            "version":   version,
        });

        let resp = state.http
            .post(format!("http://{}/agent/setup_runtime", agent_addr))
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("jotti-agent setup_runtime call failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text   = resp.text().await.unwrap_or_default();
            anyhow::bail!("jotti-agent setup_runtime returned {}: {}", status, text);
        }

        tracing::info!(site_id = %site_id, %runtime, "Remote runtime provisioning OK");
    } else {
        // Local provisioning — jotti-agent runs on the same machine.
        // The actual setup logic lives in jotti-agent; here we call the
        // runtime handler directly via its public API if running as a monolith,
        // or via the loopback gRPC address for the local agent.
        let agent_addr = std::env::var("ORBIT_AGENT_LOCAL_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:7443".into());

        let body = serde_json::json!({
            "site_id":   site_id,
            "domain":    domain,
            "unix_user": unix_user,
            "runtime":   runtime,
            "version":   version,
        });

        let resp = state.http
            .post(format!("http://{}/agent/setup_runtime", agent_addr))
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("local jotti-agent setup_runtime call failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text   = resp.text().await.unwrap_or_default();
            anyhow::bail!("local jotti-agent setup_runtime returned {}: {}", status, text);
        }

        tracing::info!(site_id = %site_id, %runtime, "Local runtime provisioning OK");
    }

    write_audit_log(
        state,
        "setup_runtime",
        &site_id.to_string(),
        &format!("domain={} runtime={} version={}", domain, runtime, version),
    )
    .await;

    Ok(())
}

/// Fire-and-forget audit log write. Errors are logged but don't propagate.
async fn write_audit_log(state: &Arc<AppState>, action: &str, target: &str, detail: &str) {
    let details = serde_json::json!({"detail": detail});
    let res = sqlx::query!(
        "INSERT INTO audit_log (action, target_name, details, ip_address, created_at)
         VALUES ($1, $2, $3, '0.0.0.0'::inet, NOW())",
        action,
        target,
        details,
    )
    .execute(&state.db)
    .await;

    if let Err(e) = res {
        tracing::warn!(error = %e, action = %action, "audit_log insert failed");
    }
}
