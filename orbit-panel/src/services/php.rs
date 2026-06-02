use std::path::PathBuf;
use uuid::Uuid;

// ── PHP extension management ──────────────────────────────────────────────────

/// Write a PHP-FPM pool .ini file with the given extension list enabled.
///
/// Extensions are added as `php_admin_value[extension=<ext>.so]`.
/// The pool file is at `/etc/php/<version>/fpm/pool.d/orbit_<domain>.conf`.
/// After writing, PHP-FPM is reloaded via `systemctl reload php<version>-fpm`.
pub async fn set_site_extensions(
    domain:      &str,
    php_version: &str,
    extensions:  &[String],
) -> anyhow::Result<()> {
    validate_php_version(php_version)?;

    let pool_path = PathBuf::from(format!(
        "/etc/php/{}/fpm/pool.d/orbit_{}.conf",
        php_version,
        sanitise_domain(domain)
    ));

    let content = build_extension_conf(domain, php_version, extensions);

    // Atomic write: write tmp then rename
    let tmp = format!("{}.tmp", pool_path.display());
    tokio::fs::write(&tmp, &content).await
        .map_err(|e| anyhow::anyhow!("write pool conf failed: {}", e))?;
    tokio::fs::rename(&tmp, &pool_path).await
        .map_err(|e| anyhow::anyhow!("rename pool conf failed: {}", e))?;

    // Reload PHP-FPM
    let service = format!("php{}-fpm", php_version);
    let status = tokio::task::spawn_blocking({
        let service = service.clone();
        move || {
            std::process::Command::new("/usr/bin/systemctl")
                .args(["reload", &service])
                .status()
        }
    })
    .await??;

    if !status.success() {
        return Err(anyhow::anyhow!("systemctl reload {} failed (exit {:?})", service, status.code()));
    }

    tracing::info!(%domain, %php_version, ext_count = extensions.len(), "PHP extensions updated");
    Ok(())
}

/// Enable JIT in the PHP-FPM pool config for this site.
pub async fn set_jit_enabled(domain: &str, php_version: &str, enabled: bool) -> anyhow::Result<()> {
    validate_php_version(php_version)?;

    let pool_path = PathBuf::from(format!(
        "/etc/php/{}/fpm/pool.d/orbit_{}_jit.ini",
        php_version,
        sanitise_domain(domain)
    ));

    let content = if enabled {
        format!(
            "; JIT enabled for {}\nphp_admin_value[opcache.jit] = 1255\nphp_admin_value[opcache.jit_buffer_size] = 64M\n",
            domain
        )
    } else {
        format!("; JIT disabled for {}\nphp_admin_value[opcache.jit] = off\n", domain)
    };

    let tmp = format!("{}.tmp", pool_path.display());
    tokio::fs::write(&tmp, &content).await?;
    tokio::fs::rename(&tmp, &pool_path).await?;

    // Reload PHP-FPM
    reload_fpm(php_version).await?;

    tracing::info!(%domain, %php_version, %enabled, "JIT setting updated");
    Ok(())
}

/// List available PHP extensions for a given version by reading the extension_dir.
pub async fn list_available_extensions(php_version: &str) -> anyhow::Result<Vec<String>> {
    validate_php_version(php_version)?;

    let ext_dir = format!("/usr/lib/php/{}", php_ext_dir(php_version));

    let mut extensions = Vec::new();
    let mut entries = tokio::fs::read_dir(&ext_dir).await
        .map_err(|e| anyhow::anyhow!("read ext_dir {} failed: {}", ext_dir, e))?;

    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        if s.ends_with(".so") {
            extensions.push(s.trim_end_matches(".so").to_string());
        }
    }

    extensions.sort();
    Ok(extensions)
}

/// List currently enabled extensions for a site (from the pool conf).
pub async fn list_enabled_extensions(domain: &str, php_version: &str) -> anyhow::Result<Vec<String>> {
    validate_php_version(php_version)?;

    let pool_path = format!(
        "/etc/php/{}/fpm/pool.d/orbit_{}.conf",
        php_version, sanitise_domain(domain)
    );

    let content = tokio::fs::read_to_string(&pool_path).await
        .map_err(|_| anyhow::anyhow!("Pool config not found for {}", domain))?;

    let enabled: Vec<String> = content.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.starts_with("php_admin_value[extension=") {
                let ext = line
                    .trim_start_matches("php_admin_value[extension=")
                    .trim_end_matches("].so]")
                    .trim_end_matches(']')
                    .to_string();
                Some(ext)
            } else {
                None
            }
        })
        .collect();

    Ok(enabled)
}

// ── PHP-FPM pool memory limit ─────────────────────────────────────────────────

pub async fn set_memory_limit(
    domain:       &str,
    php_version:  &str,
    memory_mb:    u32,
) -> anyhow::Result<()> {
    validate_php_version(php_version)?;

    if memory_mb < 32 || memory_mb > 4096 {
        return Err(anyhow::anyhow!("memory_mb must be 32–4096"));
    }

    let pool_path = format!(
        "/etc/php/{}/fpm/pool.d/orbit_{}_mem.ini",
        php_version, sanitise_domain(domain)
    );

    let content = format!(
        "; Memory limit for {}\nphp_admin_value[memory_limit] = {}M\n",
        domain, memory_mb
    );

    let tmp = format!("{}.tmp", pool_path);
    tokio::fs::write(&tmp, &content).await?;
    tokio::fs::rename(&tmp, &pool_path).await?;

    reload_fpm(php_version).await?;

    tracing::info!(%domain, %php_version, memory_mb, "PHP memory limit updated");
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn build_extension_conf(domain: &str, php_version: &str, extensions: &[String]) -> String {
    let mut conf = format!(
        "; PHP extensions for {} (PHP {})\n; Managed by OrbitCP — do not edit manually\n[{}]\n",
        domain, php_version, sanitise_domain(domain)
    );

    for ext in extensions {
        // Validate extension names: only alphanumeric + underscore + dash
        if ext.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
            conf.push_str(&format!("php_admin_value[extension={}.so]\n", ext));
        } else {
            tracing::warn!(%ext, "Skipping extension with invalid name");
        }
    }

    conf
}

fn validate_php_version(v: &str) -> anyhow::Result<()> {
    let allowed = ["7.4", "8.0", "8.1", "8.2", "8.3", "8.4"];
    if !allowed.contains(&v) {
        return Err(anyhow::anyhow!("Unsupported PHP version: {}", v));
    }
    Ok(())
}

/// Extension directory suffix by PHP version (distro-specific).
fn php_ext_dir(version: &str) -> &'static str {
    match version {
        "7.4" => "20190902",
        "8.0" => "20200930",
        "8.1" => "20210902",
        "8.2" => "20220829",
        "8.3" => "20230831",
        "8.4" => "20240924",
        _     => "20230831", // default to 8.3
    }
}

async fn reload_fpm(php_version: &str) -> anyhow::Result<()> {
    let service = format!("php{}-fpm", php_version);
    let status = tokio::task::spawn_blocking({
        let service = service.clone();
        move || {
            std::process::Command::new("/usr/bin/systemctl")
                .args(["reload", &service])
                .status()
        }
    })
    .await??;

    anyhow::ensure!(status.success(), "systemctl reload {} failed", service);
    Ok(())
}

fn sanitise_domain(domain: &str) -> String {
    domain.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
        .collect()
}
