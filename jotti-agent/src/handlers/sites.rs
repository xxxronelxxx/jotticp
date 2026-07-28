//! Site lifecycle handlers: CreateSite, DeleteSite, SuspendSite, UnsuspendSite.

use crate::journal::Journal;
use crate::proto::{
    CreateSiteRequest, DeleteSiteRequest, SiteResponse, StatusResponse, SuspendSiteRequest,
    UnsuspendSiteRequest,
};
use crate::validation::{unix_user_for_domain, validate_domain, validate_php_version, validate_web_server};
use chrono::Utc;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use tonic::Status;
use tracing::{error, info, warn};

// ── CreateSite ────────────────────────────────────────────────────────────────

pub async fn create_site(
    req: CreateSiteRequest,
    journal: &Journal,
) -> Result<tonic::Response<SiteResponse>, Status> {
    let domain     = validate_domain(&req.domain)?;
    let php_ver    = validate_php_version(&req.php_version)?;
    let _web_server = validate_web_server(&req.web_server)?;
    let unix_user  = unix_user_for_domain(&domain);

    info!(%domain, %unix_user, php = %php_ver, web = %req.web_server, "create_site: start");

    // Journal the operation before touching the filesystem
    let op_id = journal
        .begin_op(
            "create_site",
            Some(&domain),
            &serde_json::json!({ "domain": domain, "unix_user": unix_user, "php_version": php_ver }),
        )
        .await
        .map_err(|e| Status::internal(format!("journal error: {}", e)))?;

    let result = do_create_site(&domain, &unix_user, &php_ver, &req.web_server, req.disk_quota_mb);

    match &result {
        Ok(_) => {
            let _ = journal.finish_op(&op_id).await;
            info!(%domain, %unix_user, "create_site: success");
        }
        Err(e) => {
            let _ = journal.fail_op(&op_id, &e.message()).await;
            error!(%domain, err = %e.message(), "create_site: failed");
        }
    }

    result.map(|unix_user| {
        tonic::Response::new(SiteResponse {
            site_id:   domain.clone(),
            status:    "active".into(),
            unix_user,
            error_msg: String::new(),
        })
    })
}

fn do_create_site(
    domain: &str,
    unix_user: &str,
    php_version: &str,
    web_server: &str,
    disk_quota_mb: i64,
) -> Result<String, Status> {
    // 1. Create system user ─────────────────────────────────────────────────
    let status = Command::new("/usr/sbin/useradd")
        .args([
            "--system",
            "--no-create-home",
            "--shell",
            "/usr/sbin/nologin",
            "--comment",
            &format!("JottiCP site {}", domain),
            unix_user,
        ])
        .status()
        .map_err(|e| Status::internal(format!("useradd exec failed: {}", e)))?;

    if !status.success() {
        // Exit code 9 = user already exists — acceptable on idempotent re-run
        if status.code() != Some(9) {
            return Err(Status::internal(format!(
                "useradd failed with exit code {:?}",
                status.code()
            )));
        }
        warn!(%unix_user, "user already exists — continuing");
    }

    // 2. Create homedir structure ────────────────────────────────────────────
    let home_dir   = format!("/home/{}", unix_user);
    let public_html = format!("{}/public_html", home_dir);

    std::fs::create_dir_all(&public_html)
        .map_err(|e| Status::internal(format!("mkdir public_html failed: {}", e)))?;

    // chown home_dir to unix_user:unix_user
    chown_path(&home_dir, unix_user)?;
    chown_path(&public_html, unix_user)?;

    // chmod 700 on home_dir — only the site user may read/traverse it.
    // 750 would allow the group (often www-data) to read other sites' files.
    std::fs::set_permissions(&home_dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| Status::internal(format!("chmod home_dir failed: {}", e)))?;

    // 3. cgroup v2 systemd slice ─────────────────────────────────────────────
    let slice_name  = format!("website-{}.slice", domain_to_slice_name(domain));
    let memory_max  = if disk_quota_mb > 0 { "512M" } else { "512M" };
    let slice_unit  = format!(
        "[Unit]\nDescription=JottiCP site cgroup slice for {domain}\n\n\
         [Slice]\nMemoryMax={memory_max}\nCPUQuota=100%\nTasksMax=64\n\
         IOWeight=100\n\n\
         [Install]\nWantedBy=slices.target\n",
        domain = domain,
        memory_max = memory_max,
    );

    let slice_path = format!("/etc/systemd/system/{}", slice_name);
    std::fs::write(&slice_path, slice_unit)
        .map_err(|e| Status::internal(format!("write slice unit failed: {}", e)))?;

    run_cmd("/bin/systemctl", &["daemon-reload"])?;
    run_cmd("/bin/systemctl", &["start", &slice_name])?;

    // 4. PHP-FPM pool ───────────────────────────────────────────────────────
    let socket_path = format!("/run/php/php{}-fpm-{}.sock", php_version, unix_user);
    let pool_conf   = super::php::render_pool(domain, unix_user, php_version, &socket_path, 10, 256);
    let pool_path   = format!("/etc/php/{}/fpm/pool.d/{}.conf", php_version, unix_user);

    if let Some(parent) = Path::new(&pool_path).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Status::internal(format!("create pool dir failed: {}", e)))?;
    }
    std::fs::write(&pool_path, pool_conf)
        .map_err(|e| Status::internal(format!("write php pool failed: {}", e)))?;

    reload_phpfpm(php_version)?;

    // 5. Web server vhost ──────────────────────────────────────────────────
    match web_server {
        "nginx" | "" => {
            super::webserver::write_nginx_vhost(domain, unix_user, php_version, &socket_path)?;
            super::webserver::reload_nginx()?;
        }
        "ols" | "lse" => {
            super::webserver::write_ols_vhost(domain, unix_user, "")?;
            super::webserver::reload_ols()?;
        }
        "apache" => {
            super::webserver::write_apache_vhost(domain, unix_user, php_version, &socket_path)?;
            super::webserver::reload_apache()?;
        }
        other => {
            warn!(%other, "unknown web server — skipping vhost");
        }
    }

    Ok(unix_user.to_string())
}

// ── DeleteSite ────────────────────────────────────────────────────────────────

pub async fn delete_site(
    req: DeleteSiteRequest,
    journal: &Journal,
) -> Result<tonic::Response<StatusResponse>, Status> {
    let domain    = validate_domain(&req.domain)?;
    let unix_user = unix_user_for_domain(&domain);

    info!(%domain, %unix_user, delete_files = req.delete_files, "delete_site: start");

    let op_id = journal
        .begin_op(
            "delete_site",
            Some(&domain),
            &serde_json::json!({ "domain": domain, "unix_user": unix_user }),
        )
        .await
        .map_err(|e| Status::internal(format!("journal error: {}", e)))?;

    let result = do_delete_site(&domain, &unix_user, req.delete_files);

    match &result {
        Ok(_)  => { let _ = journal.finish_op(&op_id).await; }
        Err(e) => { let _ = journal.fail_op(&op_id, &e.message()).await; }
    }

    result
}

fn do_delete_site(
    domain: &str,
    unix_user: &str,
    delete_files: bool,
) -> Result<tonic::Response<StatusResponse>, Status> {
    let slice_name = format!("website-{}.slice", domain_to_slice_name(domain));
    let slice_path = format!("/etc/systemd/system/{}", slice_name);

    // 1. Backup site files before deletion ─────────────────────────────────
    let home_dir = format!("/home/{}", unix_user);
    if delete_files && Path::new(&home_dir).exists() {
        let ts        = Utc::now().format("%Y%m%d%H%M%S");
        let backup_dir = "/var/backups/jotticp/deleted";
        std::fs::create_dir_all(backup_dir)
            .map_err(|e| Status::internal(format!("create backup dir failed: {}", e)))?;

        let archive_path = format!("{}/{}-{}.tar.gz", backup_dir, domain, ts);
        let status = Command::new("/bin/tar")
            .args(["-czf", &archive_path, "-C", "/home", unix_user])
            .status()
            .map_err(|e| Status::internal(format!("tar exec failed: {}", e)))?;
        if !status.success() {
            warn!(%domain, "backup tar failed — continuing with delete anyway");
        } else {
            info!(%domain, archive = %archive_path, "site files backed up");
        }
    }

    // 2. Stop + disable cgroup slice ────────────────────────────────────────
    let _ = Command::new("/bin/systemctl").args(["stop", &slice_name]).status();
    let _ = Command::new("/bin/systemctl").args(["disable", &slice_name]).status();
    let _ = std::fs::remove_file(&slice_path);
    let _ = Command::new("/bin/systemctl").args(["daemon-reload"]).status();

    // 3. Remove PHP-FPM pools for any installed PHP versions ────────────────
    for ver in ["8.1", "8.2", "8.3", "8.4"] {
        let pool_path = format!("/etc/php/{}/fpm/pool.d/{}.conf", ver, unix_user);
        if Path::new(&pool_path).exists() {
            let _ = std::fs::remove_file(&pool_path);
            let _ = reload_phpfpm(ver);
        }
    }

    // 4. Remove nginx vhost ─────────────────────────────────────────────────
    let nginx_avail  = format!("/etc/nginx/sites-available/{}.conf", domain);
    let nginx_enabled = format!("/etc/nginx/sites-enabled/{}.conf", domain);
    let _ = std::fs::remove_file(&nginx_enabled);
    let _ = std::fs::remove_file(&nginx_avail);
    let _ = Command::new("/usr/sbin/nginx").args(["-s", "reload"]).status();

    // Remove OLS vhost
    let ols_vhost_dir = format!("/usr/local/lsws/conf/vhosts/{}", domain);
    let _ = std::fs::remove_dir_all(&ols_vhost_dir);

    // Remove Apache vhost
    let apache_vhost = format!("/etc/apache2/sites-available/{}.conf", domain);
    let _ = Command::new("/usr/sbin/a2dissite").arg(&format!("{}.conf", domain)).status();
    let _ = std::fs::remove_file(&apache_vhost);

    // 5. Delete Unix user ───────────────────────────────────────────────────
    let _ = Command::new("/usr/sbin/userdel")
        .args(["--remove", unix_user])
        .status();

    // 6. Remove home directory ──────────────────────────────────────────────
    if delete_files {
        let _ = std::fs::remove_dir_all(&format!("/home/{}", unix_user));
    }

    info!(%domain, %unix_user, "delete_site: success");

    Ok(tonic::Response::new(StatusResponse {
        success: true,
        message: format!("site {} deleted", domain),
    }))
}

// ── SuspendSite ───────────────────────────────────────────────────────────────

pub async fn suspend_site(
    req: SuspendSiteRequest,
    journal: &Journal,
) -> Result<tonic::Response<StatusResponse>, Status> {
    let domain    = validate_domain(&req.domain)?;
    let unix_user = unix_user_for_domain(&domain);

    info!(%domain, "suspend_site: start");

    let op_id = journal
        .begin_op("suspend_site", Some(&domain), &serde_json::json!({ "domain": domain }))
        .await
        .map_err(|e| Status::internal(format!("journal error: {}", e)))?;

    let result = do_suspend_site(&domain, &unix_user);

    match &result {
        Ok(_)  => { let _ = journal.finish_op(&op_id).await; }
        Err(e) => { let _ = journal.fail_op(&op_id, &e.message()).await; }
    }

    result
}

fn do_suspend_site(domain: &str, unix_user: &str) -> Result<tonic::Response<StatusResponse>, Status> {
    let slice_name = format!("website-{}.slice", domain_to_slice_name(domain));

    // 1. Set nginx vhost to return 503 ─────────────────────────────────────
    let suspended_conf = nginx_suspended_vhost(domain);
    let vhost_path     = format!("/etc/nginx/sites-available/{}.conf", domain);

    // Save original if not already suspended
    let backup_path = format!("/etc/nginx/sites-available/{}.conf.presuspend", domain);
    if Path::new(&vhost_path).exists() && !Path::new(&backup_path).exists() {
        let _ = std::fs::copy(&vhost_path, &backup_path);
    }

    if Path::new(&vhost_path).exists() {
        std::fs::write(&vhost_path, suspended_conf)
            .map_err(|e| Status::internal(format!("write suspended vhost failed: {}", e)))?;
        let _ = Command::new("/usr/sbin/nginx").args(["-s", "reload"]).status();
    }

    // 2. Stop PHP-FPM pool ─────────────────────────────────────────────────
    for ver in ["8.1", "8.2", "8.3", "8.4"] {
        let pool_path = format!("/etc/php/{}/fpm/pool.d/{}.conf", ver, unix_user);
        if Path::new(&pool_path).exists() {
            // Move pool to disabled location to stop it from being loaded
            let disabled_path = format!("/etc/php/{}/fpm/pool.d/{}.conf.suspended", ver, unix_user);
            let _ = std::fs::rename(&pool_path, &disabled_path);
            let _ = reload_phpfpm(ver);
        }
    }

    // 3. Freeze cgroup slice ───────────────────────────────────────────────
    // NOTE: Do NOT set MemoryMax=0 — in cgroup v2 that means 0 bytes of
    // allowed memory and would instantly OOM every process in the slice.
    // The correct approach is SIGSTOP to pause all processes, which is
    // reversible via SIGCONT without data loss.
    let _ = Command::new("/bin/systemctl")
        .args(["kill", "--signal=SIGSTOP", &slice_name])
        .status();

    info!(%domain, "suspend_site: success");

    Ok(tonic::Response::new(StatusResponse {
        success: true,
        message: format!("site {} suspended", domain),
    }))
}

// ── UnsuspendSite ─────────────────────────────────────────────────────────────

pub async fn unsuspend_site(
    req: UnsuspendSiteRequest,
    journal: &Journal,
) -> Result<tonic::Response<StatusResponse>, Status> {
    let domain    = validate_domain(&req.domain)?;
    let unix_user = unix_user_for_domain(&domain);

    info!(%domain, "unsuspend_site: start");

    let op_id = journal
        .begin_op("unsuspend_site", Some(&domain), &serde_json::json!({ "domain": domain }))
        .await
        .map_err(|e| Status::internal(format!("journal error: {}", e)))?;

    let result = do_unsuspend_site(&domain, &unix_user);

    match &result {
        Ok(_)  => { let _ = journal.finish_op(&op_id).await; }
        Err(e) => { let _ = journal.fail_op(&op_id, &e.message()).await; }
    }

    result
}

fn do_unsuspend_site(domain: &str, unix_user: &str) -> Result<tonic::Response<StatusResponse>, Status> {
    let slice_name = format!("website-{}.slice", domain_to_slice_name(domain));

    // 1. Restore nginx vhost ───────────────────────────────────────────────
    let vhost_path   = format!("/etc/nginx/sites-available/{}.conf", domain);
    let backup_path  = format!("/etc/nginx/sites-available/{}.conf.presuspend", domain);

    if Path::new(&backup_path).exists() {
        std::fs::rename(&backup_path, &vhost_path)
            .map_err(|e| Status::internal(format!("restore vhost failed: {}", e)))?;
        let _ = Command::new("/usr/sbin/nginx").args(["-s", "reload"]).status();
    }

    // 2. Restore PHP-FPM pool ─────────────────────────────────────────────
    for ver in ["8.1", "8.2", "8.3", "8.4"] {
        let pool_path     = format!("/etc/php/{}/fpm/pool.d/{}.conf", ver, unix_user);
        let disabled_path = format!("/etc/php/{}/fpm/pool.d/{}.conf.suspended", ver, unix_user);
        if Path::new(&disabled_path).exists() {
            let _ = std::fs::rename(&disabled_path, &pool_path);
            let _ = reload_phpfpm(ver);
        }
    }

    // 3. Unfreeze cgroup slice ─────────────────────────────────────────────
    let _ = Command::new("/bin/systemctl")
        .args(["kill", "--signal=SIGCONT", &slice_name])
        .status();

    // Restore memory limit to 512M (default)
    let _ = Command::new("/bin/systemctl")
        .args(["set-property", &slice_name, "MemoryMax=512M"])
        .status();

    info!(%domain, "unsuspend_site: success");

    Ok(tonic::Response::new(StatusResponse {
        success: true,
        message: format!("site {} unsuspended", domain),
    }))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Convert a domain name to a safe systemd slice name component.
/// "sub.example.com" → "sub-example-com"
pub fn domain_to_slice_name(domain: &str) -> String {
    domain.replace('.', "-").replace('_', "-")
}

fn chown_path(path: &str, user: &str) -> Result<(), Status> {
    let status = Command::new("/bin/chown")
        .args(["-R", &format!("{}:{}", user, user), path])
        .status()
        .map_err(|e| Status::internal(format!("chown exec failed: {}", e)))?;

    if !status.success() {
        return Err(Status::internal(format!(
            "chown {} failed with exit code {:?}",
            path,
            status.code()
        )));
    }
    Ok(())
}

pub fn reload_phpfpm(php_version: &str) -> Result<(), Status> {
    // Validate php_version before using it in a command arg
    let _ = validate_php_version(php_version)?;
    let service = format!("php{}-fpm", php_version);
    run_cmd("/bin/systemctl", &["reload-or-restart", &service])
}

pub fn run_cmd(binary: &str, args: &[&str]) -> Result<(), Status> {
    let status = Command::new(binary)
        .args(args)
        .status()
        .map_err(|e| Status::internal(format!("exec {} failed: {}", binary, e)))?;

    if !status.success() {
        return Err(Status::internal(format!(
            "{} {:?} failed with exit code {:?}",
            binary,
            args,
            status.code()
        )));
    }
    Ok(())
}

fn nginx_suspended_vhost(domain: &str) -> String {
    format!(
        r#"server {{
    listen 80;
    listen [::]:80;
    server_name {domain} www.{domain};

    location / {{
        return 503;
    }}

    error_page 503 /503.html;
    location = /503.html {{
        root /var/www/jotticp-suspended;
        internal;
    }}
}}
"#,
        domain = domain,
    )
}
