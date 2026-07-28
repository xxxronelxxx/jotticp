mod client;
mod tools;
mod transport;

use anyhow::Result;
use client::OrbitClient;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "jotti_mcp=info".into()),
        )
        .init();

    let use_http = std::env::args().any(|a| a == "--http");

    let orbit_client = OrbitClient::from_env()
        .map_err(|e| anyhow::anyhow!("Failed to initialise OrbitClient: {}", e))?;
    let client = Arc::new(Mutex::new(orbit_client));

    if use_http {
        tracing::info!("Starting jotti-mcp in HTTP mode on 127.0.0.1:3002");
        transport::http::run_http(client).await?;
    } else {
        tracing::info!("Starting jotti-mcp in stdio mode");
        transport::stdio::run_stdio(client).await?;
    }

    Ok(())
}

// ── Tool dispatcher ───────────────────────────────────────────────────────────

/// Route a `tools/call` invocation to the appropriate OrbitClient method.
pub async fn dispatch_tool(
    name: &str,
    args: &Value,
    client: Arc<Mutex<OrbitClient>>,
) -> Result<Value> {
    let c = client.lock().await;

    match name {
        "list_sites" => {
            let search = args.get("search").and_then(|v| v.as_str());
            c.list_sites(search).await
        }
        "create_site" => {
            c.create_site(args).await
        }
        "delete_site" => {
            let site_id = args.get("site_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("site_id is required"))?;
            c.delete_site(site_id).await
        }
        "get_site_stats" => {
            let site_id = args.get("site_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("site_id is required"))?;
            c.get_site_stats(site_id).await
        }
        "list_email_accounts" => {
            let domain = args.get("domain").and_then(|v| v.as_str());
            c.list_email_accounts(domain).await
        }
        "create_email_account" => {
            c.create_email_account(args).await
        }
        "delete_email_account" => {
            let account_id = args.get("account_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("account_id is required"))?;
            c.delete_email_account(account_id).await
        }
        "list_dns_zones" => {
            let search = args.get("search").and_then(|v| v.as_str());
            c.list_dns_zones(search).await
        }
        "create_dns_record" => {
            let zone_id = args.get("zone_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("zone_id is required"))?;
            c.create_dns_record(zone_id, args).await
        }
        "list_ssl_certs" => {
            let expiring_soon = args.get("expiring_soon")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            c.list_ssl_certs(expiring_soon).await
        }
        "renew_ssl" => {
            let cert_id = args.get("cert_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("cert_id is required"))?;
            c.renew_ssl(cert_id).await
        }
        "list_databases" => {
            let site_id = args.get("site_id").and_then(|v| v.as_str());
            c.list_databases(site_id).await
        }
        "create_database" => {
            c.create_database(args).await
        }
        "trigger_backup" => {
            let site_id = args.get("site_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("site_id is required"))?;
            let body = serde_json::json!({
                "type": args.get("type").and_then(|v| v.as_str()).unwrap_or("full")
            });
            c.trigger_backup(site_id, &body).await
        }
        "get_server_stats" => {
            let server_id = args.get("server_id").and_then(|v| v.as_str());
            c.get_server_stats(server_id).await
        }
        "install_app" => {
            c.install_app(args).await
        }
        "list_installed_apps" => {
            let site_id = args.get("site_id").and_then(|v| v.as_str());
            c.list_installed_apps(site_id).await
        }
        "list_cron_jobs" => {
            let site_id = args.get("site_id").and_then(|v| v.as_str());
            c.list_cron_jobs(site_id).await
        }
        "create_cron_job" => {
            c.create_cron_job(args).await
        }
        "get_system_logs" => {
            let limit = args.get("limit")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);
            let action = args.get("action").and_then(|v| v.as_str());
            let user_id = args.get("user_id").and_then(|v| v.as_str());
            c.get_system_logs(limit, action, user_id).await
        }
        other => anyhow::bail!("Unknown tool: {}", other),
    }
}
