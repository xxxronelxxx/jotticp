//! PHP-FPM pool management handlers: CreatePhpFpmPool, DeletePhpFpmPool, ReloadPhpFpmPool.

use crate::journal::Journal;
use crate::proto::{DeletePhpFpmPoolRequest, PhpFpmPoolRequest, ReloadPhpFpmPoolRequest, StatusResponse};
use crate::validation::{unix_user_for_domain, validate_domain, validate_php_version, validate_unix_user};
use std::path::Path;
use tonic::Status;
use tracing::{info, warn};

// ── Pool config template ──────────────────────────────────────────────────────

const POOL_TEMPLATE: &str = r#"[{domain}]
user  = {unix_user}
group = {unix_user}

listen = {socket_path}
listen.owner = www-data
listen.group = www-data
listen.mode  = 0660

pm                        = ondemand
pm.max_children           = {max_children}
pm.process_idle_timeout   = 30s
pm.max_requests           = 500

; Security: restrict file access to site home + shared PHP dirs
php_admin_value[open_basedir]       = /home/{unix_user}:/tmp:/usr/share/php:/usr/lib/php
php_admin_value[disable_functions]  = exec,passthru,shell_exec,system,proc_open,popen,proc_nice,proc_terminate,proc_get_status,dl,show_source,eval,pcntl_exec
php_admin_flag[expose_php]          = off
php_admin_value[memory_limit]       = {memory_limit_mb}M
php_admin_value[error_log]          = /var/log/php-fpm/{unix_user}-error.log
php_admin_flag[log_errors]          = on
php_admin_value[upload_max_filesize] = 64M
php_admin_value[post_max_size]       = 64M

; OPcache — enabled per site
php_admin_value[opcache.enable]              = 1
php_admin_value[opcache.memory_consumption]  = 64
php_admin_value[opcache.max_accelerated_files] = 4000
php_admin_value[opcache.revalidate_freq]     = 2
php_admin_value[opcache.jit_buffer_size]     = 32M
php_admin_value[opcache.jit]                 = 1255

; Environment
env[PATH]     = /usr/local/bin:/usr/bin:/bin
env[TMPDIR]   = /tmp
"#;

// ── render_pool ───────────────────────────────────────────────────────────────

/// Render a PHP-FPM pool configuration string from the template.
pub fn render_pool(
    domain: &str,
    unix_user: &str,
    _php_version: &str,
    socket_path: &str,
    max_children: i32,
    memory_limit_mb: i64,
) -> String {
    let max_children    = if max_children <= 0 { 10 } else { max_children };
    let memory_limit_mb = if memory_limit_mb <= 0 { 256 } else { memory_limit_mb };

    POOL_TEMPLATE
        .replace("{domain}", domain)
        .replace("{unix_user}", unix_user)
        .replace("{socket_path}", socket_path)
        .replace("{max_children}", &max_children.to_string())
        .replace("{memory_limit_mb}", &memory_limit_mb.to_string())
}

// ── CreatePhpFpmPool ──────────────────────────────────────────────────────────

pub async fn create_php_fpm_pool(
    req: PhpFpmPoolRequest,
    journal: &Journal,
) -> Result<tonic::Response<StatusResponse>, Status> {
    let domain    = validate_domain(&req.domain)?;
    let php_ver   = validate_php_version(&req.php_version)?;
    let unix_user = if req.unix_user.is_empty() {
        unix_user_for_domain(&domain)
    } else {
        validate_unix_user(&req.unix_user)?
    };

    // Derive socket path if caller did not provide one
    let socket_path = if req.socket_path.is_empty() {
        format!("/run/php/php{}-fpm-{}.sock", php_ver, unix_user)
    } else {
        req.socket_path.clone()
    };

    info!(%domain, %unix_user, php = %php_ver, "create_php_fpm_pool");

    let op_id = journal
        .begin_op(
            "create_php_fpm_pool",
            Some(&domain),
            &serde_json::json!({ "domain": domain, "php_version": php_ver, "unix_user": unix_user }),
        )
        .await
        .map_err(|e| Status::internal(format!("journal error: {}", e)))?;

    let result = do_create_pool(&domain, &unix_user, &php_ver, &socket_path, req.max_children, req.memory_limit_mb);

    match &result {
        Ok(_)  => { let _ = journal.finish_op(&op_id).await; }
        Err(e) => { let _ = journal.fail_op(&op_id, &e.message()).await; }
    }

    result
}

fn do_create_pool(
    domain: &str,
    unix_user: &str,
    php_version: &str,
    socket_path: &str,
    max_children: i32,
    memory_limit_mb: i64,
) -> Result<tonic::Response<StatusResponse>, Status> {
    let pool_conf = render_pool(domain, unix_user, php_version, socket_path, max_children, memory_limit_mb);
    let pool_path = format!("/etc/php/{}/fpm/pool.d/{}.conf", php_version, unix_user);

    // Ensure log directory exists
    let log_dir = format!("/var/log/php-fpm");
    let _ = std::fs::create_dir_all(&log_dir);

    // Ensure pool.d directory exists
    if let Some(parent) = Path::new(&pool_path).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Status::internal(format!("create pool.d dir failed: {}", e)))?;
    }

    std::fs::write(&pool_path, &pool_conf)
        .map_err(|e| Status::internal(format!("write php pool failed: {}", e)))?;

    super::sites::reload_phpfpm(php_version)?;

    info!(pool_path = %pool_path, php = %php_version, "PHP-FPM pool created");

    Ok(tonic::Response::new(StatusResponse {
        success: true,
        message: format!("PHP-FPM pool {} created for PHP {}", domain, php_version),
    }))
}

// ── DeletePhpFpmPool ──────────────────────────────────────────────────────────

pub async fn delete_php_fpm_pool(
    req: DeletePhpFpmPoolRequest,
    journal: &Journal,
) -> Result<tonic::Response<StatusResponse>, Status> {
    let domain  = validate_domain(&req.domain)?;
    let php_ver = validate_php_version(&req.php_version)?;
    let unix_user = unix_user_for_domain(&domain);

    info!(%domain, php = %php_ver, "delete_php_fpm_pool");

    let op_id = journal
        .begin_op(
            "delete_php_fpm_pool",
            Some(&domain),
            &serde_json::json!({ "domain": domain, "php_version": php_ver }),
        )
        .await
        .map_err(|e| Status::internal(format!("journal error: {}", e)))?;

    let pool_path = format!("/etc/php/{}/fpm/pool.d/{}.conf", php_ver, unix_user);

    if Path::new(&pool_path).exists() {
        if let Err(e) = std::fs::remove_file(&pool_path) {
            let _ = journal.fail_op(&op_id, &e.to_string()).await;
            return Err(Status::internal(format!("remove pool file failed: {}", e)));
        }

        if let Err(e) = super::sites::reload_phpfpm(&php_ver) {
            let _ = journal.fail_op(&op_id, &e.message()).await;
            return Err(e);
        }
    } else {
        warn!(%pool_path, "pool config not found — nothing to delete");
    }

    let _ = journal.finish_op(&op_id).await;

    Ok(tonic::Response::new(StatusResponse {
        success: true,
        message: format!("PHP-FPM pool {} deleted for PHP {}", domain, php_ver),
    }))
}

// ── ReloadPhpFpmPool ──────────────────────────────────────────────────────────

pub async fn reload_php_fpm_pool(
    req: ReloadPhpFpmPoolRequest,
) -> Result<tonic::Response<StatusResponse>, Status> {
    let _domain  = validate_domain(&req.domain)?;
    let php_ver  = validate_php_version(&req.php_version)?;

    super::sites::reload_phpfpm(&php_ver)?;

    Ok(tonic::Response::new(StatusResponse {
        success: true,
        message: format!("PHP {} FPM reloaded", php_ver),
    }))
}
