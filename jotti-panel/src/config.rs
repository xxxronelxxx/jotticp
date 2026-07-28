use anyhow::Context;
use axum::http::header::HeaderValue;
use jsonwebtoken::{DecodingKey, EncodingKey};
use tower_http::cors::AllowOrigin;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrbitEnv {
    Community,
    Pro,
    Enterprise,
}

impl OrbitEnv {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "pro"        => Self::Pro,
            "enterprise" => Self::Enterprise,
            _            => Self::Community,
        }
    }
    pub fn is_pro_or_above(&self) -> bool {
        matches!(self, Self::Pro | Self::Enterprise)
    }
}

impl std::fmt::Display for OrbitEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Community  => write!(f, "community"),
            Self::Pro        => write!(f, "pro"),
            Self::Enterprise => write!(f, "enterprise"),
        }
    }
}

/// Raw config loaded from env — does NOT contain pre-built JWT keys (those live on AppState).
#[derive(Clone, Debug)]
pub struct Config {
    pub database_url:            String,
    /// Path to ECDSA P-256 private key PEM — EncodingKey built from this at startup.
    pub jwt_ec_private_key_path: String,
    /// Path to ECDSA P-256 public key PEM — DecodingKey built from this at startup.
    pub jwt_ec_public_key_path:  String,
    pub orbit_env:               OrbitEnv,
    pub admin_email:             String,
    pub panel_port:              u16,
    pub valkey_url:              String,
    pub allowed_origins:         Vec<String>,
    pub acme_directory:          String,
    pub max_upload_mb:           u64,
    pub panel_domain:            String,
    /// Base URL for the webmail UI (e.g. https://jotticp.waytohosts.com/webmail).
    /// Used when generating SSO login links. Set via WEBMAIL_URL env var.
    pub webmail_url:             String,
    pub smtp_from:               String,
    /// Base URL for the internal jotti-mail daemon (default: http://127.0.0.1:5354)
    pub orbit_mail_url:          String,
    /// Base URL for the internal jotti-dns daemon (default: http://127.0.0.1:5353)
    pub orbit_dns_url:           String,
    /// Shared secret used to authenticate internal-to-panel calls (e.g. git post-receive hook).
    /// Loaded from ORBIT_INTERNAL_TOKEN env var. Optional — if unset, the record endpoint
    /// will reject all calls (deploy recording is silently skipped by the hook).
    pub internal_token:          Option<String>,
    /// Alias for orbit_mail_url — used by email_ips.rs handler.
    /// Always equal to orbit_mail_url.
    pub mail_base_url:           String,
    /// Base URL for the jotti-agent gRPC/HTTP control endpoint on managed servers.
    /// Defaults to http://127.0.0.1:7080 (local loopback for single-server installs).
    /// In multi-server deployments each request should look up the per-server address.
    pub agent_base_url:          String,
    /// Maximum number of databases allowed per site. Default: 10.
    pub max_databases_per_site:  u32,
    /// Maximum number of cron jobs allowed per site. Default: 25.
    pub max_cron_jobs_per_site:  u32,
    /// Maximum number of email accounts allowed per domain. Default: 100.
    pub max_email_accounts_per_domain: u32,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let database_url = std::env::var("DATABASE_URL")
            .context("DATABASE_URL is required")?;

        let jwt_ec_private_key_path = std::env::var("JWT_EC_PRIVATE_KEY_PATH")
            .unwrap_or_else(|_| "/etc/jotticp/jwt_ec_key.pem".into());

        let jwt_ec_public_key_path = std::env::var("JWT_EC_PUBLIC_KEY_PATH")
            .unwrap_or_else(|_| "/etc/jotticp/jwt_ec_key.pub.pem".into());

        let orbit_env = OrbitEnv::from_str(
            &std::env::var("ORBIT_ENV").unwrap_or_else(|_| "community".into()),
        );

        let admin_email = std::env::var("ADMIN_EMAIL")
            .unwrap_or_else(|_| "admin@localhost".into());

        let panel_port = std::env::var("PANEL_PORT")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(2087);

        let valkey_url = std::env::var("VALKEY_URL")
            .unwrap_or_else(|_| "redis://127.0.0.1:6379".into());

        let allowed_origins = std::env::var("ALLOWED_ORIGINS")
            .unwrap_or_else(|_| "https://localhost".into())
            .split(',').map(|s| s.trim().to_string()).collect();

        let acme_directory = std::env::var("ACME_DIRECTORY")
            .unwrap_or_else(|_| "https://acme-v02.api.letsencrypt.org/directory".into());

        let max_upload_mb = std::env::var("MAX_UPLOAD_MB")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(50);

        let panel_domain = std::env::var("PANEL_DOMAIN")
            .unwrap_or_else(|_| "localhost".into());

        let webmail_url = std::env::var("WEBMAIL_URL")
            .unwrap_or_else(|_| format!("https://webmail.{}", panel_domain));

        let smtp_from = std::env::var("SMTP_FROM")
            .unwrap_or_else(|_| format!("noreply@{}", panel_domain));

        let orbit_mail_url = std::env::var("ORBIT_MAIL_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:5354".into());

        let orbit_dns_url = std::env::var("ORBIT_DNS_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:5353".into());

        let internal_token = std::env::var("ORBIT_INTERNAL_TOKEN").ok();

        let agent_base_url = std::env::var("ORBIT_AGENT_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:7080".into());

        // mail_base_url is an alias for orbit_mail_url used by agent-generated modules.
        let mail_base_url = orbit_mail_url.clone();

        let max_databases_per_site = std::env::var("ORBIT_MAX_DATABASES_PER_SITE")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(10);

        let max_cron_jobs_per_site = std::env::var("ORBIT_MAX_CRON_JOBS_PER_SITE")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(25);

        let max_email_accounts_per_domain = std::env::var("ORBIT_MAX_EMAIL_ACCOUNTS")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(100);

        Ok(Self {
            database_url,
            jwt_ec_private_key_path,
            jwt_ec_public_key_path,
            orbit_env,
            admin_email,
            panel_port,
            valkey_url,
            allowed_origins,
            acme_directory,
            max_upload_mb,
            panel_domain,
            webmail_url,
            smtp_from,
            orbit_mail_url,
            orbit_dns_url,
            internal_token,
            mail_base_url,
            agent_base_url,
            max_databases_per_site,
            max_cron_jobs_per_site,
            max_email_accounts_per_domain,
        })
    }

    /// Load EC keys from disk and build JWT EncodingKey + DecodingKey.
    /// Called once at startup. Keys are stored on AppState, not re-read per request.
    pub fn load_jwt_keys(&self) -> anyhow::Result<(EncodingKey, DecodingKey)> {
        // Generate keys if not present (first run)
        if !std::path::Path::new(&self.jwt_ec_private_key_path).exists() {
            tracing::info!("JWT EC keys not found — generating new P-256 keypair");
            Self::generate_ec_keypair(&self.jwt_ec_private_key_path, &self.jwt_ec_public_key_path)?;
        }

        let private_pem = std::fs::read(&self.jwt_ec_private_key_path)
            .with_context(|| format!("Cannot read JWT private key: {}", self.jwt_ec_private_key_path))?;
        let public_pem = std::fs::read(&self.jwt_ec_public_key_path)
            .with_context(|| format!("Cannot read JWT public key: {}", self.jwt_ec_public_key_path))?;

        let encoding_key = EncodingKey::from_ec_pem(&private_pem)
            .context("Invalid EC private key PEM")?;
        let decoding_key = DecodingKey::from_ec_pem(&public_pem)
            .context("Invalid EC public key PEM")?;

        Ok((encoding_key, decoding_key))
    }

    /// Generate a fresh ECDSA P-256 keypair using openssl CLI (always available on target OS).
    fn generate_ec_keypair(private_path: &str, public_path: &str) -> anyhow::Result<()> {
        // Ensure directory exists
        if let Some(dir) = std::path::Path::new(private_path).parent() {
            std::fs::create_dir_all(dir).ok();
        }

        // Generate PKCS8 EC private key (P-256) — jsonwebtoken requires PKCS8 PEM
        let status = std::process::Command::new("openssl")
            .args(["genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256", "-out", private_path])
            .status()
            .context("openssl not found — cannot generate JWT keys")?;
        anyhow::ensure!(status.success(), "openssl genpkey failed");

        // Extract public key
        let status = std::process::Command::new("openssl")
            .args(["pkey", "-in", private_path, "-pubout", "-out", public_path])
            .status()?;
        anyhow::ensure!(status.success(), "openssl pkey pubout failed");

        // Restrict private key permissions to owner-read-only
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(private_path, std::fs::Permissions::from_mode(0o600))?;
        }

        tracing::info!("JWT EC keypair generated at {}", private_path);
        Ok(())
    }

    pub fn allowed_origins(&self) -> AllowOrigin {
        let origins: Vec<HeaderValue> = self.allowed_origins.iter()
            .filter_map(|o| o.parse::<HeaderValue>().ok()).collect();
        match origins.len() {
            0 => AllowOrigin::exact("https://localhost".parse().unwrap()),
            1 => AllowOrigin::exact(origins.into_iter().next().unwrap()),
            _ => AllowOrigin::list(origins),
        }
    }

    pub fn max_upload_bytes(&self) -> u64 { self.max_upload_mb * 1024 * 1024 }
    pub fn is_pro(&self) -> bool { self.orbit_env.is_pro_or_above() }
}
