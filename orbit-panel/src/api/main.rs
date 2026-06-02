use axum::{Router, routing::get};
use jsonwebtoken::{EncodingKey, DecodingKey};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tower_http::{cors::CorsLayer, trace::TraceLayer, compression::CompressionLayer};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

mod api;
mod config;
mod db;
mod error;
mod middleware;
mod services;

pub use error::{ApiError, ApiResult};

/// Shared application state — Arc-wrapped, cloned cheaply per request.
#[derive(Clone)]
pub struct AppState {
    pub db:                PgPool,
    pub config:            config::Config,
    pub valkey:            redis::aio::ConnectionManager,
    /// ES256 ECDSA P-256 signing key — built from /etc/orbitcp/jwt_ec_key.pem at startup.
    pub jwt_encoding_key:  EncodingKey,
    /// ES256 ECDSA P-256 verification key — built from /etc/orbitcp/jwt_ec_key.pub.pem at startup.
    pub jwt_decoding_key:  DecodingKey,
    /// Shared reqwest HTTP client for calling internal daemons (orbit-mail, orbit-dns).
    pub http:              reqwest::Client,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    #[cfg(not(feature = "console"))]
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "orbit_panel=info,tower_http=info".into()))
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    dotenvy::dotenv().ok();
    let config = config::Config::from_env()?;
    tracing::info!(version = env!("CARGO_PKG_VERSION"), env = %config.orbit_env, "orbit-panel starting");

    // Load or generate EC keypair for JWT ES256 signing
    let (jwt_encoding_key, jwt_decoding_key) = config.load_jwt_keys()?;
    tracing::info!("JWT ES256 keys loaded");

    // PostgreSQL connection pool
    let db = PgPoolOptions::new()
        .max_connections(32).min_connections(4)
        .acquire_timeout(Duration::from_secs(5))
        .idle_timeout(Duration::from_secs(600))
        .connect(&config.database_url).await
        .map_err(|e| anyhow::anyhow!("PostgreSQL connect failed: {}", e))?;

    // Run migrations on startup
    sqlx::migrate!("./migrations").run(&db).await
        .map_err(|e| anyhow::anyhow!("Migration failed: {}", e))?;
    tracing::info!("PostgreSQL connected, migrations applied");

    // Valkey (Redis-compatible) for session blocklist + job queue + cache
    let valkey_client = redis::Client::open(config.valkey_url.as_str())
        .map_err(|e| anyhow::anyhow!("Valkey client error: {}", e))?;
    let valkey = redis::aio::ConnectionManager::new(valkey_client).await
        .map_err(|e| anyhow::anyhow!("Valkey connect failed: {}", e))?;
    tracing::info!("Valkey connected");

    // Internal HTTP client for orbit-mail / orbit-dns daemon calls
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| anyhow::anyhow!("reqwest build failed: {}", e))?;

    let state = Arc::new(AppState {
        db, config: config.clone(), valkey, jwt_encoding_key, jwt_decoding_key, http,
    });

    // Start background job processor (backup, SSL renewal, provisioning)
    services::jobs::start_job_processor(state.clone());

    // Build the Axum router
    let app = Router::new()
        .route("/health",       get(health_check))
        .route("/health/ready", get(readiness_check))
        // Auth
        .merge(api::auth::router())
        // Resources
        .merge(api::sites::router())
        .merge(api::databases::router())
        .merge(api::dbmanager::router())
        .merge(api::email::router())
        .merge(api::dns::router())
        .merge(api::ssl::router())
        .merge(api::filemanager::router())
        .merge(api::php::router())
        .merge(api::cache::router())
        .merge(api::cron::router())
        .merge(api::backups::router())
        .merge(api::apps::router())
        .merge(api::runtimes::router())
        // Security
        .merge(api::firewall::router())
        .merge(api::fail2ban::router())
        .merge(api::waf::router())
        .merge(api::ssh_keys::router())
        // Infrastructure
        .merge(api::servers::router())
        .merge(api::users::router())
        .merge(api::notifications::router())
        .merge(api::settings::router())
        .merge(api::branding::router())
        // Git push-to-deploy
        .merge(api::git_deploy::router())
        // Staging environments (Pro — nested under /api/v1/sites)
        .nest("/api/v1/sites", api::staging::router())
        // Reseller email IP pools
        .merge(api::email_ips::router())
        // Client portal (non-admin self-service)
        .nest("/api/v1/client",    api::client::router())
        // Reseller management
        .nest("/api/v1/reseller",  api::reseller::router())
        // cPanel migration import
        .nest("/api/v1/migration", api::migration::router())
        // Realtime
        .merge(api::ws::router())
        // Middleware (applied bottom-up)
        .layer(axum::middleware::from_fn(middleware::security_headers::security_headers))
        // Audit log: must wrap routes so it can see the JWT and response status.
        // Applied AFTER security_headers so the audit itself is not double-counted.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::audit::audit_log,
        ))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(
            CorsLayer::new()
                .allow_origin(config.allowed_origins())
                .allow_methods([
                    axum::http::Method::GET, axum::http::Method::POST,
                    axum::http::Method::PUT, axum::http::Method::DELETE,
                    axum::http::Method::OPTIONS,
                ])
                .allow_headers([
                    axum::http::header::AUTHORIZATION,
                    axum::http::header::CONTENT_TYPE,
                    axum::http::header::ACCEPT,
                ])
                .allow_credentials(true),
        )
        .with_state(state.clone());

    // orbit-panel binds to 127.0.0.1 ONLY — nginx/OLS reverse-proxies to :443
    let addr = SocketAddr::from(([127, 0, 0, 1], config.panel_port));
    tracing::info!(%addr, "orbit-panel listening");

    let listener = tokio::net::TcpListener::bind(addr).await
        .map_err(|e| anyhow::anyhow!("Bind failed {}: {}", addr, e))?;

    // ConnectInfo::<SocketAddr> is used by login / verify_totp for IP logging.
    // `into_make_service_with_connect_info` is required — without it the extractor
    // returns a missing-extension error at runtime.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .map_err(|e| anyhow::anyhow!("Server error: {}", e))?;

    tracing::info!("orbit-panel shut down gracefully");
    Ok(())
}

async fn health_check() -> &'static str { "ok" }

async fn readiness_check(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> impl axum::response::IntoResponse {
    match sqlx::query("SELECT 1").execute(&state.db).await {
        Ok(_)  => (axum::http::StatusCode::OK, "ready"),
        Err(_) => (axum::http::StatusCode::SERVICE_UNAVAILABLE, "not ready"),
    }
}

async fn shutdown_signal() {
    let ctrl_c = async { tokio::signal::ctrl_c().await.expect("ctrl-c handler") };
    #[cfg(unix)]
    let term = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("sigterm handler").recv().await;
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => tracing::info!("Ctrl-C"),
        _ = term   => tracing::info!("SIGTERM"),
    }
}
