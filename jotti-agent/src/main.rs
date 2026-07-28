use std::net::SocketAddr;
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub mod proto {
    tonic::include_proto!("jotti");
}

pub mod handlers;

pub mod journal;
pub mod validation;

use handlers::{cgroups, database, dns, files, metrics, php, runtime, sites, system, webserver};
// backup, quota, ssl are used by other handlers internally
#[allow(unused_imports)]
use handlers::{backup, quota, ssl};
use proto::agent_server::{Agent, AgentServer};
use proto::*;

// ── AgentService ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AgentService {
    pub journal: std::sync::Arc<journal::Journal>,
}

#[tonic::async_trait]
impl Agent for AgentService {
    // ── Site lifecycle ────────────────────────────────────────────────────────

    async fn create_site(
        &self,
        request: tonic::Request<CreateSiteRequest>,
    ) -> Result<tonic::Response<SiteResponse>, tonic::Status> {
        let req = request.into_inner();
        sites::create_site(req, &self.journal).await
    }

    async fn delete_site(
        &self,
        request: tonic::Request<DeleteSiteRequest>,
    ) -> Result<tonic::Response<StatusResponse>, tonic::Status> {
        let req = request.into_inner();
        sites::delete_site(req, &self.journal).await
    }

    async fn suspend_site(
        &self,
        request: tonic::Request<SuspendSiteRequest>,
    ) -> Result<tonic::Response<StatusResponse>, tonic::Status> {
        let req = request.into_inner();
        sites::suspend_site(req, &self.journal).await
    }

    async fn unsuspend_site(
        &self,
        request: tonic::Request<UnsuspendSiteRequest>,
    ) -> Result<tonic::Response<StatusResponse>, tonic::Status> {
        let req = request.into_inner();
        sites::unsuspend_site(req, &self.journal).await
    }

    // ── PHP-FPM pool management ───────────────────────────────────────────────

    async fn create_php_fpm_pool(
        &self,
        request: tonic::Request<PhpFpmPoolRequest>,
    ) -> Result<tonic::Response<StatusResponse>, tonic::Status> {
        let req = request.into_inner();
        php::create_php_fpm_pool(req, &self.journal).await
    }

    async fn delete_php_fpm_pool(
        &self,
        request: tonic::Request<DeletePhpFpmPoolRequest>,
    ) -> Result<tonic::Response<StatusResponse>, tonic::Status> {
        let req = request.into_inner();
        php::delete_php_fpm_pool(req, &self.journal).await
    }

    async fn reload_php_fpm_pool(
        &self,
        request: tonic::Request<ReloadPhpFpmPoolRequest>,
    ) -> Result<tonic::Response<StatusResponse>, tonic::Status> {
        let req = request.into_inner();
        php::reload_php_fpm_pool(req).await
    }

    // ── cgroup v2 resource limits ─────────────────────────────────────────────

    async fn set_cgroup_limits(
        &self,
        request: tonic::Request<CgroupLimitsRequest>,
    ) -> Result<tonic::Response<StatusResponse>, tonic::Status> {
        let req = request.into_inner();
        cgroups::set_cgroup_limits(req, &self.journal).await
    }

    async fn get_cgroup_stats(
        &self,
        request: tonic::Request<GetCgroupStatsRequest>,
    ) -> Result<tonic::Response<CgroupStatsResponse>, tonic::Status> {
        let req = request.into_inner();
        cgroups::get_cgroup_stats(req).await
    }

    // ── Web server management ─────────────────────────────────────────────────

    async fn reload_web_server(
        &self,
        request: tonic::Request<ReloadWebServerRequest>,
    ) -> Result<tonic::Response<StatusResponse>, tonic::Status> {
        let req = request.into_inner();
        webserver::reload_web_server(req).await
    }

    async fn get_site_stats(
        &self,
        request: tonic::Request<GetSiteStatsRequest>,
    ) -> Result<tonic::Response<SiteStatsResponse>, tonic::Status> {
        let req = request.into_inner();
        metrics::get_site_stats(req).await
    }

    // ── Database management ───────────────────────────────────────────────────

    async fn create_database(
        &self,
        request: tonic::Request<CreateDatabaseRequest>,
    ) -> Result<tonic::Response<DatabaseResponse>, tonic::Status> {
        let req = request.into_inner();
        database::create_database(req, &self.journal).await
    }

    async fn delete_database(
        &self,
        request: tonic::Request<DeleteDatabaseRequest>,
    ) -> Result<tonic::Response<StatusResponse>, tonic::Status> {
        let req = request.into_inner();
        database::delete_database(req, &self.journal).await
    }

    // ── DNS zone management ───────────────────────────────────────────────────

    async fn create_dns_zone(
        &self,
        request: tonic::Request<CreateDnsZoneRequest>,
    ) -> Result<tonic::Response<StatusResponse>, tonic::Status> {
        let req = request.into_inner();
        dns::create_dns_zone(req, &self.journal).await
    }

    async fn delete_dns_zone(
        &self,
        request: tonic::Request<DeleteDnsZoneRequest>,
    ) -> Result<tonic::Response<StatusResponse>, tonic::Status> {
        let req = request.into_inner();
        dns::delete_dns_zone(req, &self.journal).await
    }

    async fn upsert_dns_record(
        &self,
        request: tonic::Request<DnsRecordRequest>,
    ) -> Result<tonic::Response<StatusResponse>, tonic::Status> {
        let req = request.into_inner();
        dns::upsert_dns_record(req, &self.journal).await
    }

    async fn delete_dns_record(
        &self,
        request: tonic::Request<DeleteDnsRecordRequest>,
    ) -> Result<tonic::Response<StatusResponse>, tonic::Status> {
        let req = request.into_inner();
        dns::delete_dns_record(req, &self.journal).await
    }

    // ── File operations ───────────────────────────────────────────────────────

    async fn extract_archive(
        &self,
        request: tonic::Request<ExtractArchiveRequest>,
    ) -> Result<tonic::Response<StatusResponse>, tonic::Status> {
        let req = request.into_inner();
        files::extract_archive(req).await
    }

    async fn run_command(
        &self,
        request: tonic::Request<RunCommandRequest>,
    ) -> Result<tonic::Response<RunCommandResponse>, tonic::Status> {
        let req = request.into_inner();
        files::run_command(req).await
    }

    // ── Streaming metrics ─────────────────────────────────────────────────────

    type StreamMetricsStream =
        std::pin::Pin<Box<dyn futures_util::Stream<Item = Result<MetricsUpdate, tonic::Status>> + Send>>;

    async fn stream_metrics(
        &self,
        request: tonic::Request<StreamMetricsRequest>,
    ) -> Result<tonic::Response<Self::StreamMetricsStream>, tonic::Status> {
        let req = request.into_inner();
        metrics::stream_metrics(req).await
    }

    // ── System information ────────────────────────────────────────────────────

    async fn get_system_info(
        &self,
        _request: tonic::Request<Empty>,
    ) -> Result<tonic::Response<SystemInfoResponse>, tonic::Status> {
        system::get_system_info().await
    }

    async fn get_disk_usage(
        &self,
        request: tonic::Request<GetDiskUsageRequest>,
    ) -> Result<tonic::Response<DiskUsageResponse>, tonic::Status> {
        let req = request.into_inner();
        system::get_disk_usage(req).await
    }

    // ── Language runtime provisioning (Steps 4.9–4.12) ───────────────────────

    async fn setup_python_site(
        &self,
        request: tonic::Request<SetupPythonSiteRequest>,
    ) -> Result<tonic::Response<StatusResponse>, tonic::Status> {
        let req = request.into_inner();
        runtime::setup_python_site(
            &req.site_id,
            &req.domain,
            &req.unix_user,
            &req.python_version,
            &self.journal,
        )
        .await
        .map(|_| tonic::Response::new(StatusResponse {
            success: true,
            message: format!("Python site {} provisioned", req.domain),
        }))
    }

    async fn setup_nodejs_site(
        &self,
        request: tonic::Request<SetupNodejsSiteRequest>,
    ) -> Result<tonic::Response<StatusResponse>, tonic::Status> {
        let req = request.into_inner();
        runtime::setup_nodejs_site(
            &req.site_id,
            &req.domain,
            &req.unix_user,
            &req.node_version,
            &self.journal,
        )
        .await
        .map(|_| tonic::Response::new(StatusResponse {
            success: true,
            message: format!("Node.js site {} provisioned", req.domain),
        }))
    }

    async fn setup_ruby_site(
        &self,
        request: tonic::Request<SetupRubySiteRequest>,
    ) -> Result<tonic::Response<StatusResponse>, tonic::Status> {
        let req = request.into_inner();
        runtime::setup_ruby_site(
            &req.site_id,
            &req.domain,
            &req.unix_user,
            &req.ruby_version,
            &self.journal,
        )
        .await
        .map(|_| tonic::Response::new(StatusResponse {
            success: true,
            message: format!("Ruby site {} provisioned", req.domain),
        }))
    }

    async fn setup_dotnet_site(
        &self,
        request: tonic::Request<SetupDotnetSiteRequest>,
    ) -> Result<tonic::Response<StatusResponse>, tonic::Status> {
        let req = request.into_inner();
        runtime::setup_dotnet_site(
            &req.site_id,
            &req.domain,
            &req.unix_user,
            &req.dotnet_version,
            &self.journal,
        )
        .await
        .map(|_| tonic::Response::new(StatusResponse {
            success: true,
            message: format!(".NET site {} provisioned", req.domain),
        }))
    }

    // ── PHP extension management ──────────────────────────────────────────────

    async fn update_php_extensions(
        &self,
        _request: tonic::Request<UpdatePhpExtensionsRequest>,
    ) -> Result<tonic::Response<StatusResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("not yet implemented"))
    }

    // ── Vhost management ──────────────────────────────────────────────────────

    async fn create_vhost(
        &self,
        _request: tonic::Request<CreateVhostRequest>,
    ) -> Result<tonic::Response<StatusResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("not yet implemented"))
    }

    async fn delete_vhost(
        &self,
        _request: tonic::Request<DeleteVhostRequest>,
    ) -> Result<tonic::Response<StatusResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("not yet implemented"))
    }

    // ── SSL certificate deployment ────────────────────────────────────────────

    async fn install_ssl_cert(
        &self,
        _request: tonic::Request<InstallSslCertRequest>,
    ) -> Result<tonic::Response<StatusResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("not yet implemented"))
    }

    async fn remove_ssl_cert(
        &self,
        _request: tonic::Request<RemoveSslCertRequest>,
    ) -> Result<tonic::Response<StatusResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("not yet implemented"))
    }

    // ── Disk quota ────────────────────────────────────────────────────────────

    async fn set_disk_quota(
        &self,
        _request: tonic::Request<SetDiskQuotaRequest>,
    ) -> Result<tonic::Response<StatusResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("not yet implemented"))
    }

    // ── Backup operations ─────────────────────────────────────────────────────

    async fn trigger_backup(
        &self,
        _request: tonic::Request<TriggerBackupRequest>,
    ) -> Result<tonic::Response<TriggerBackupResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("not yet implemented"))
    }

    async fn get_backup_status(
        &self,
        _request: tonic::Request<GetBackupStatusRequest>,
    ) -> Result<tonic::Response<BackupStatusResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("not yet implemented"))
    }

    // ── Connectivity test ─────────────────────────────────────────────────────

    async fn ping(
        &self,
        _request: tonic::Request<Empty>,
    ) -> Result<tonic::Response<PingResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("not yet implemented"))
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Rustls 0.23 requires an explicit default crypto provider when more than one
    // (aws-lc-rs + ring) is compiled in — otherwise the mTLS setup panics at startup.
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .ok();

    // Initialise tracing with JSON output (production-friendly, parsed by log aggregators)
    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "jotti_agent=info,tonic=warn".into()),
        )
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    dotenvy::dotenv().ok();

    let addr: SocketAddr = std::env::var("ORBIT_AGENT_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:7443".into())
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid ORBIT_AGENT_ADDR: {}", e))?;

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        %addr,
        "jotti-agent starting"
    );

    // ── mTLS configuration ────────────────────────────────────────────────────
    let cert_path = std::env::var("AGENT_CERT_PATH")
        .unwrap_or_else(|_| "/etc/jotticp/agent/agent.pem".into());
    let key_path = std::env::var("AGENT_KEY_PATH")
        .unwrap_or_else(|_| "/etc/jotticp/agent/agent-key.pem".into());
    let ca_path = std::env::var("AGENT_CA_PATH")
        .unwrap_or_else(|_| "/etc/jotticp/agent/ca.pem".into());

    let cert = std::fs::read_to_string(&cert_path)
        .map_err(|e| anyhow::anyhow!("Cannot read agent cert {}: {}", cert_path, e))?;
    let key = std::fs::read_to_string(&key_path)
        .map_err(|e| anyhow::anyhow!("Cannot read agent key {}: {}", key_path, e))?;
    let ca_cert = std::fs::read_to_string(&ca_path)
        .map_err(|e| anyhow::anyhow!("Cannot read CA cert {}: {}", ca_path, e))?;

    let identity = Identity::from_pem(cert, key);
    let client_ca = Certificate::from_pem(ca_cert);
    let tls_config = ServerTlsConfig::new()
        .identity(identity)
        .client_ca_root(client_ca); // mutual TLS — client MUST present a cert

    // ── SQLite journal ────────────────────────────────────────────────────────
    let journal_path = std::env::var("ORBIT_JOURNAL_PATH")
        .unwrap_or_else(|_| "/var/lib/jotticp/agent/journal.db".into());

    // Ensure journal directory exists
    if let Some(parent) = std::path::Path::new(&journal_path).parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            anyhow::anyhow!("Cannot create journal dir {:?}: {}", parent, e)
        })?;
    }

    let journal = journal::Journal::new(&journal_path)
        .await
        .map_err(|e| anyhow::anyhow!("Cannot open journal: {}", e))?;

    let journal_arc = std::sync::Arc::new(journal);

    // ── Service ───────────────────────────────────────────────────────────────
    let service = AgentService {
        journal: journal_arc,
    };

    tracing::info!("jotti-agent listening with mTLS on {}", addr);

    Server::builder()
        .tls_config(tls_config)?
        .add_service(AgentServer::new(service))
        .serve_with_shutdown(addr, shutdown_signal())
        .await
        .map_err(|e| anyhow::anyhow!("gRPC server error: {}", e))?;

    tracing::info!("jotti-agent shutdown complete");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c    => { tracing::info!("received SIGINT — shutting down"); },
        _ = terminate => { tracing::info!("received SIGTERM — shutting down"); },
    }
}
