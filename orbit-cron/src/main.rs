/// orbit-cron — background scheduler daemon
///
/// Runs four independent Tokio tasks:
///   Task 1 (every 1h):    SSL expiry check → push orbit:jobs
///   Task 2 (daily 3 AM):  Backup execution → push orbit:jobs
///   Task 3 (every 6h):    DNSBL blacklist check → insert blacklist_alerts
///   Task 4 (every 30s):   Collect metrics from orbit-agent via gRPC
///
/// Connects to PostgreSQL and Valkey from environment variables.
/// Terminates cleanly on SIGTERM.

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use chrono::{Local, NaiveTime};
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use sqlx::PgPool;
use tokio::sync::Mutex as TokioMutex;
use tracing::{error, info, warn};

// ─────────────────────────────────────────────────────────────────────────────
// gRPC generated code (orbit.proto)
// ─────────────────────────────────────────────────────────────────────────────

pub mod orbit_proto {
    tonic::include_proto!("orbit");
}

use orbit_proto::{agent_client::AgentClient, StreamMetricsRequest};

// ─────────────────────────────────────────────────────────────────────────────
// Configuration
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Config {
    database_url: String,
    valkey_url: String,
    /// Comma-separated list of "server_id:grpc_url" pairs
    agent_endpoints: Vec<(String, String)>,
    /// SMTP relay for alert emails (optional; alerts are DB-only if absent)
    admin_email: Option<String>,
}

impl Config {
    fn from_env() -> Result<Self> {
        let database_url =
            std::env::var("DATABASE_URL").context("DATABASE_URL env var is required")?;
        let valkey_url = std::env::var("VALKEY_URL")
            .or_else(|_| std::env::var("REDIS_URL"))
            .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

        // AGENT_ENDPOINTS=server1:https://...,server2:https://...
        let agent_endpoints: Vec<(String, String)> = std::env::var("AGENT_ENDPOINTS")
            .unwrap_or_default()
            .split(',')
            .filter(|s| !s.is_empty())
            .filter_map(|entry| {
                let mut parts = entry.splitn(2, ':');
                let server_id = parts.next()?.trim().to_string();
                let url = parts.next()?.trim().to_string();
                Some((server_id, url))
            })
            .collect();

        let admin_email = std::env::var("ADMIN_ALERT_EMAIL").ok();

        Ok(Self {
            database_url,
            valkey_url,
            agent_endpoints,
            admin_email,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared state
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct AppState {
    db:     PgPool,
    valkey: ConnectionManager,
    config: Arc<Config>,
    /// Guards against concurrent metrics collection rounds.
    /// A `TokioMutex<()>` so each server's collection is still independent
    /// (we spawn per-server), but a new *round* won't start until the
    /// previous round's spawned tasks have all finished.
    metrics_lock: Arc<TokioMutex<()>>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 1: SSL expiry check (every 1 hour)
// ─────────────────────────────────────────────────────────────────────────────

async fn task_ssl_expiry(state: AppState) {
    // Base interval: 1 hour.  On rate-limit errors we back off exponentially
    // up to a cap of 6 hours before retrying, then reset to the base.
    const BASE_SECS: u64 = 3600;
    const MAX_BACKOFF_SECS: u64 = 6 * 3600;
    let mut backoff_secs: u64 = 0; // 0 means "use base interval"

    loop {
        let sleep_secs = if backoff_secs > 0 { backoff_secs } else { BASE_SECS };
        info!(sleep_secs, "ssl_expiry: sleeping");
        tokio::time::sleep(Duration::from_secs(sleep_secs)).await;
        info!("ssl_expiry: running check");

        match ssl_expiry_check(&state).await {
            Ok(count) => {
                info!(jobs_queued = count, "ssl_expiry: check complete");
                backoff_secs = 0; // reset backoff on success
            }
            Err(e) => {
                let msg = e.to_string();
                // Detect Let's Encrypt / ACME rate-limit signals.
                let is_rate_limited = msg.contains("rateLimited")
                    || msg.contains("rate limit")
                    || msg.contains("429")
                    || msg.contains("too many");

                if is_rate_limited {
                    let next = (if backoff_secs == 0 { BASE_SECS } else { backoff_secs * 2 })
                        .min(MAX_BACKOFF_SECS);
                    backoff_secs = next;
                    warn!(
                        error = %e,
                        next_retry_secs = backoff_secs,
                        "ssl_expiry: rate-limited by Let's Encrypt, backing off"
                    );
                } else {
                    error!(error = %e, "ssl_expiry: check failed");
                    backoff_secs = 0; // non-rate-limit errors: try again next hour
                }
            }
        }
    }
}

async fn ssl_expiry_check(state: &AppState) -> Result<usize> {
    struct CertRow {
        site_id: Option<sqlx::types::Uuid>,
        domain: String,
    }

    let rows = sqlx::query_as!(
        CertRow,
        r#"
        SELECT site_id, domain
        FROM ssl_certs
        WHERE expires_at < NOW() + INTERVAL '30 days'
          AND auto_renew = true
        "#
    )
    .fetch_all(&state.db)
    .await
    .context("query ssl_certs")?;

    let mut count = 0usize;
    let mut conn = state.valkey.clone();

    for row in &rows {
        let site_id_str = match row.site_id {
            Some(id) => id.to_string(),
            None => continue,
        };
        let job = serde_json::json!({
            "type": "issue_ssl",
            "site_id": site_id_str,
            "domain": row.domain,
        });
        let payload = serde_json::to_string(&job).unwrap();

        conn.lpush::<_, _, ()>("orbit:jobs", &payload)
            .await
            .with_context(|| format!("lpush ssl job for {}", row.domain))?;

        info!(domain = %row.domain, site_id = %site_id_str, "queued ssl renewal job");
        count += 1;
    }

    Ok(count)
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 2: Backup execution (daily at 3 AM)
// ─────────────────────────────────────────────────────────────────────────────

async fn task_backup(state: AppState) {
    loop {
        // Calculate seconds until next 03:00:00 local time
        let now = Local::now();
        let target = NaiveTime::from_hms_opt(3, 0, 0).unwrap();
        let now_time = now.time();
        let secs_until_3am = if now_time < target {
            (target - now_time).num_seconds() as u64
        } else {
            // Already past 3am today, wait until 3am tomorrow
            (86400 - (now_time - target).num_seconds()) as u64
        };

        info!(secs = secs_until_3am, "backup: sleeping until 3 AM");
        tokio::time::sleep(Duration::from_secs(secs_until_3am)).await;

        info!("backup: running daily backup job scheduling");
        match backup_check(&state).await {
            Ok(count) => info!(jobs_queued = count, "backup: scheduling complete"),
            Err(e) => error!(error = %e, "backup: scheduling failed"),
        }

        // Sleep 1 minute to avoid re-triggering immediately
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

async fn backup_check(state: &AppState) -> Result<usize> {
    struct SiteRow {
        site_id: sqlx::types::Uuid,
    }

    // Sites that have backup_settings configured (any entry = daily backup enabled)
    let rows = sqlx::query_as!(
        SiteRow,
        r#"
        SELECT s.id AS site_id
        FROM sites s
        JOIN backup_settings bs ON bs.site_id = s.id
        WHERE s.status::text = 'active'
        "#
    )
    .fetch_all(&state.db)
    .await
    .context("query backup sites")?;

    let mut count = 0usize;
    let mut conn = state.valkey.clone();

    for row in &rows {
        let job = serde_json::json!({
            "type": "backup_site",
            "site_id": row.site_id.to_string(),
        });
        let payload = serde_json::to_string(&job).unwrap();

        conn.lpush::<_, _, ()>("orbit:jobs", &payload)
            .await
            .with_context(|| format!("lpush backup job for site {}", row.site_id))?;

        info!(site_id = %row.site_id, "queued backup job");
        count += 1;
    }

    Ok(count)
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 3: DNSBL blacklist check (every 6 hours)
// ─────────────────────────────────────────────────────────────────────────────

const DNSBLS: &[&str] = &[
    "zen.spamhaus.org",
    "dnsbl.sorbs.net",
    "b.barracudacentral.org",
];

async fn task_dnsbl(state: AppState) {
    let mut interval = tokio::time::interval(Duration::from_secs(6 * 3600));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        interval.tick().await;
        info!("dnsbl: running check");

        match dnsbl_check(&state).await {
            Ok(listed) => info!(listed_count = listed, "dnsbl: check complete"),
            Err(e) => error!(error = %e, "dnsbl: check failed"),
        }
    }
}

/// Reverse an IPv4 address: "1.2.3.4" → "4.3.2.1"
fn reverse_ipv4(ip: &str) -> Option<String> {
    let parts: Vec<&str> = ip.split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    Some(format!(
        "{}.{}.{}.{}",
        parts[3], parts[2], parts[1], parts[0]
    ))
}

async fn is_listed_in_dnsbl(
    resolver: &hickory_resolver::TokioAsyncResolver,
    reversed_ip: &str,
    dnsbl: &str,
) -> bool {
    let query = format!("{reversed_ip}.{dnsbl}.");
    match resolver.lookup_ip(query.as_str()).await {
        Ok(resp) => {
            let listed = resp.iter().next().is_some();
            listed
        }
        Err(e) => {
            // NXDOMAIN = not listed; other errors = treat as not listed
            if !format!("{e}").contains("NXDOMAIN") && !format!("{e}").contains("NoRecordsFound") {
                warn!(query = %query, error = %e, "DNSBL lookup error");
            }
            false
        }
    }
}

async fn dnsbl_check(state: &AppState) -> Result<usize> {
    use hickory_resolver::{config::*, TokioAsyncResolver};

    let resolver =
        TokioAsyncResolver::tokio(ResolverConfig::default(), ResolverOpts::default());

    struct DomainRow {
        domain: String,
        server_ip: Option<String>,
    }

    let rows = sqlx::query_as!(
        DomainRow,
        r#"
        SELECT s.domain, srv.ip_address::text AS server_ip
        FROM sites s
        JOIN servers srv ON srv.id = s.server_id
        WHERE s.status::text = 'active'
        "#
    )
    .fetch_all(&state.db)
    .await
    .context("query active domains for DNSBL")?;

    let mut listed_count = 0usize;

    for row in &rows {
        let ip = match &row.server_ip {
            Some(ip) => ip.clone(),
            None => {
                warn!(domain = %row.domain, "no server IP, skipping DNSBL check");
                continue;
            }
        };

        let reversed = match reverse_ipv4(&ip) {
            Some(r) => r,
            None => {
                warn!(ip = %ip, "failed to reverse IP for DNSBL check");
                continue;
            }
        };

        let ip_network: sqlx::types::ipnetwork::IpNetwork = match ip.parse() {
            Ok(n) => n,
            Err(_) => {
                warn!(ip = %ip, "invalid IP format, skipping DNSBL insert");
                continue;
            }
        };

        for dnsbl in DNSBLS {
            if is_listed_in_dnsbl(&resolver, &reversed, dnsbl).await {
                warn!(
                    domain = %row.domain,
                    ip = %ip,
                    dnsbl,
                    "IP listed in DNSBL"
                );

                // INSERT … ON CONFLICT DO NOTHING returns rows_affected=0 if
                // the row already existed, letting us distinguish new vs known.
                let inserted = sqlx::query!(
                    r#"
                    INSERT INTO blacklist_alerts (domain, ip, dnsbl, listed_at, notified_at)
                    VALUES ($1, $2, $3, NOW(), NULL)
                    ON CONFLICT (domain, ip, dnsbl) DO NOTHING
                    "#,
                    row.domain,
                    ip_network,
                    *dnsbl
                )
                .execute(&state.db)
                .await
                .with_context(|| format!("insert blacklist_alert for {}", row.domain))?
                .rows_affected();

                // Send admin email only for new alerts (not repeated ones).
                if inserted > 0 {
                    if let Some(admin_email) = &state.config.admin_email {
                        send_dnsbl_alert_email(
                            admin_email,
                            &row.domain,
                            &ip,
                            dnsbl,
                        )
                        .await;

                        // Mark as notified so we don't re-email on future runs.
                        let _ = sqlx::query!(
                            "UPDATE blacklist_alerts SET notified_at = NOW()
                             WHERE domain = $1 AND ip = $2 AND dnsbl = $3",
                            row.domain, ip_network, *dnsbl
                        )
                        .execute(&state.db)
                        .await;
                    } else {
                        warn!(
                            domain = %row.domain,
                            ip = %ip,
                            dnsbl,
                            "ADMIN_ALERT_EMAIL not set — DNSBL alert stored in DB only"
                        );
                    }
                }

                listed_count += 1;
            } else {
                info!(
                    domain = %row.domain,
                    ip = %ip,
                    dnsbl,
                    "not listed"
                );
            }
        }
    }

    Ok(listed_count)
}

/// Send a plain-text DNSBL listing alert to the admin via the `sendmail`
/// binary.  This is deliberately simple (no SMTP crate dependency needed):
/// it shells out to sendmail which honours the system MTA configuration.
///
/// If sendmail is unavailable or fails the error is logged but not propagated —
/// the DB record is the authoritative alert store.
async fn send_dnsbl_alert_email(to: &str, domain: &str, ip: &str, dnsbl: &str) {
    let subject = format!("OrbitCP DNSBL Alert: {} ({}) listed in {}", domain, ip, dnsbl);
    let body = format!(
        "OrbitCP DNSBL Check Alert\n\
         ─────────────────────────\n\
         Domain : {domain}\n\
         IP     : {ip}\n\
         DNSBL  : {dnsbl}\n\
         Action : Review and remediate. Once delisted, the alert will\n\
                  not re-fire until the IP is listed again.\n"
    );
    let message = format!(
        "To: {to}\nSubject: {subject}\nContent-Type: text/plain\n\n{body}"
    );

    let result = tokio::process::Command::new("sendmail")
        .arg("-t")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();

    match result {
        Ok(mut child) => {
            if let Some(mut stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                let _ = stdin.write_all(message.as_bytes()).await;
                drop(stdin);
            }
            if let Err(e) = child.wait().await {
                warn!(error = %e, "sendmail wait() failed for DNSBL alert");
            } else {
                info!(to, domain, dnsbl, "DNSBL alert email sent");
            }
        }
        Err(e) => {
            warn!(error = %e, "failed to spawn sendmail for DNSBL alert (DB record still written)");
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 4: Metrics collection from orbit-agent (every 30 seconds)
// ─────────────────────────────────────────────────────────────────────────────

async fn task_metrics(state: AppState) {
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    // Skip ticks that fire while the previous round is still running, rather
    // than queuing them up — prevents unbounded pileup if collection is slow.
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        interval.tick().await;

        if state.config.agent_endpoints.is_empty() {
            continue;
        }

        // Try to acquire the round lock without blocking the timer loop.
        // If the previous round is still in progress we skip this tick entirely.
        let _guard = match state.metrics_lock.try_lock() {
            Ok(g) => g,
            Err(_) => {
                warn!("metrics: previous collection round still running, skipping this tick");
                continue;
            }
        };

        // Spawn per-server collection tasks and collect their handles so we
        // can await them all before releasing the round lock.
        let handles: Vec<_> = state
            .config
            .agent_endpoints
            .iter()
            .map(|(server_id, endpoint)| {
                let server_id = server_id.clone();
                let endpoint  = endpoint.clone();
                let state     = state.clone();
                tokio::spawn(async move {
                    if let Err(e) =
                        collect_metrics_from_agent(&state, &server_id, &endpoint).await
                    {
                        warn!(server_id = %server_id, error = %e, "metrics collection failed");
                    }
                })
            })
            .collect();

        // Wait for all per-server tasks before dropping the lock.
        for h in handles {
            let _ = h.await;
        }

        // Prune old metrics (keep only last 24h)
        if let Err(e) = prune_old_metrics(&state).await {
            warn!(error = %e, "failed to prune old server_metrics");
        }
        // _guard drops here, releasing the round lock.
    }
}

async fn collect_metrics_from_agent(
    state: &AppState,
    server_id: &str,
    endpoint: &str,
) -> Result<()> {
    let server_uuid: sqlx::types::Uuid = server_id
        .parse()
        .with_context(|| format!("server_id '{server_id}' is not a valid UUID"))?;

    let mut client = AgentClient::connect(endpoint.to_string())
        .await
        .with_context(|| format!("connect to agent at {endpoint}"))?;

    let mut stream = client
        .stream_metrics(StreamMetricsRequest { interval_ms: 5000 })
        .await
        .context("stream_metrics RPC")?
        .into_inner();

    // Collect one batch of updates (up to 3 samples in 30s)
    let mut samples = 0usize;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(25);

    loop {
        tokio::select! {
            msg = stream.message() => {
                match msg {
                    Ok(Some(update)) => {
                        sqlx::query!(
                            r#"
                            INSERT INTO server_metrics
                                (server_id, recorded_at, cpu_pct, ram_pct,
                                 ram_used_bytes, ram_total_bytes,
                                 disk_read_kbps, disk_write_kbps,
                                 net_rx_kbps, net_tx_kbps,
                                 load_1, load_5, load_15)
                            VALUES
                                ($1, to_timestamp($2::bigint / 1000.0),
                                 $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                            "#,
                            server_uuid,
                            update.timestamp_ms,
                            update.cpu_pct as f64,
                            update.ram_pct as f64,
                            update.ram_used_bytes as i64,
                            update.ram_total_bytes as i64,
                            update.disk_read_kbps as i64,
                            update.disk_write_kbps as i64,
                            update.net_rx_kbps as i64,
                            update.net_tx_kbps as i64,
                            update.load_1 as f64,
                            update.load_5 as f64,
                            update.load_15 as f64,
                        )
                        .execute(&state.db)
                        .await
                        .with_context(|| format!("insert server_metrics for {server_id}"))?;

                        samples += 1;
                        if samples >= 3 {
                            break;
                        }
                    }
                    Ok(None) => break, // stream ended
                    Err(e) => {
                        warn!(server_id, error = %e, "stream_metrics error");
                        break;
                    }
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                break; // deadline reached
            }
        }
    }

    info!(server_id, samples, "metrics collected");
    Ok(())
}

async fn prune_old_metrics(state: &AppState) -> Result<()> {
    let rows_deleted = sqlx::query!(
        r#"
        DELETE FROM server_metrics
        WHERE recorded_at < NOW() - INTERVAL '24 hours'
        "#
    )
    .execute(&state.db)
    .await
    .context("prune server_metrics")?
    .rows_affected();

    if rows_deleted > 0 {
        info!(rows_deleted, "pruned old server_metrics");
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Main
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("orbit_cron=info".parse().unwrap()),
        )
        .init();

    info!("orbit-cron starting");

    let config = Config::from_env().context("failed to load configuration")?;

    // PostgreSQL connection pool
    let db = PgPool::connect(&config.database_url)
        .await
        .context("failed to connect to PostgreSQL")?;

    // Valkey / Redis connection manager
    let valkey_client =
        redis::Client::open(config.valkey_url.as_str()).context("failed to create Redis client")?;
    let valkey = ConnectionManager::new(valkey_client)
        .await
        .context("failed to connect to Valkey")?;

    let state = AppState {
        db,
        valkey,
        config:       Arc::new(config),
        metrics_lock: Arc::new(TokioMutex::new(())),
    };

    info!("all connections established, spawning tasks");

    let t1 = tokio::spawn(task_ssl_expiry(state.clone()));
    let t2 = tokio::spawn(task_backup(state.clone()));
    let t3 = tokio::spawn(task_dnsbl(state.clone()));
    let t4 = tokio::spawn(task_metrics(state.clone()));

    // Wait for SIGTERM or any task to exit unexpectedly
    tokio::select! {
        _ = shutdown_signal() => {
            info!("orbit-cron received shutdown signal, stopping");
        }
        result = t1 => {
            error!("ssl_expiry task exited unexpectedly: {result:?}");
        }
        result = t2 => {
            error!("backup task exited unexpectedly: {result:?}");
        }
        result = t3 => {
            error!("dnsbl task exited unexpectedly: {result:?}");
        }
        result = t4 => {
            error!("metrics task exited unexpectedly: {result:?}");
        }
    }

    info!("orbit-cron stopped");
    Ok(())
}

async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut sigterm = signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
    let mut sigint = signal(SignalKind::interrupt()).expect("failed to install SIGINT handler");
    tokio::select! {
        _ = sigterm.recv() => { info!("received SIGTERM"); }
        _ = sigint.recv()  => { info!("received SIGINT"); }
    }
}
