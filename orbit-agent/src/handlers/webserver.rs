//! Web server vhost management: create/delete vhosts, reload web server.

use crate::proto::{ReloadWebServerRequest, StatusResponse};
use crate::validation::{validate_domain, validate_php_version, validate_unix_user};
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;
use tonic::Status;
use tracing::{info, warn};

// ── nginx vhost ───────────────────────────────────────────────────────────────

/// Write an nginx vhost config for a site.
pub fn write_nginx_vhost(
    domain: &str,
    unix_user: &str,
    php_version: &str,
    socket_path: &str,
) -> Result<(), Status> {
    let conf_dir = "/etc/nginx/sites-available";
    std::fs::create_dir_all(conf_dir)
        .map_err(|e| Status::internal(format!("create nginx sites-available failed: {}", e)))?;

    let conf = nginx_vhost_template(domain, unix_user, php_version, socket_path);
    let conf_path = format!("{}/{}.conf", conf_dir, domain);

    std::fs::write(&conf_path, &conf)
        .map_err(|e| Status::internal(format!("write nginx vhost failed: {}", e)))?;

    // Symlink to sites-enabled
    let enabled_dir = "/etc/nginx/sites-enabled";
    std::fs::create_dir_all(enabled_dir)
        .map_err(|e| Status::internal(format!("create nginx sites-enabled failed: {}", e)))?;

    let link_path    = format!("{}/{}.conf", enabled_dir, domain);
    let _            = std::fs::remove_file(&link_path); // remove stale link if present

    symlink(&conf_path, &link_path)
        .map_err(|e| Status::internal(format!("symlink nginx sites-enabled failed: {}", e)))?;

    // Validate config before reloading
    validate_nginx_config()?;

    info!(%domain, conf_path = %conf_path, "nginx vhost created");
    Ok(())
}

fn nginx_vhost_template(
    domain: &str,
    unix_user: &str,
    php_version: &str,
    _socket_path: &str,
) -> String {
    format!(
        r#"server {{
    listen 80;
    listen [::]:80;
    server_name {domain} www.{domain};
    root /home/{unix_user}/public_html;
    index index.php index.html index.htm;

    # Security headers
    add_header X-Content-Type-Options nosniff always;
    add_header X-Frame-Options SAMEORIGIN always;
    add_header Referrer-Policy strict-origin always;

    location / {{
        try_files $uri $uri/ /index.php?$query_string;
    }}

    location ~ \.php$ {{
        try_files $uri =404;
        include fastcgi_params;
        fastcgi_pass unix:/run/php/php{php_version}-fpm-{unix_user}.sock;
        fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;
        fastcgi_param PHP_VALUE "error_log=/var/log/nginx/{domain}-php.log\n";
        fastcgi_read_timeout 300;
        fastcgi_buffer_size 16k;
        fastcgi_buffers 4 16k;
    }}

    location ~ /\.ht {{
        deny all;
    }}

    location ~ /\.git {{
        deny all;
    }}

    # Static asset caching
    location ~* \.(css|js|png|jpg|jpeg|gif|ico|svg|woff|woff2|ttf|eot)$ {{
        expires 1y;
        add_header Cache-Control "public, immutable";
        access_log off;
    }}

    access_log /var/log/nginx/{domain}.access.log combined;
    error_log  /var/log/nginx/{domain}.error.log warn;
}}
"#,
        domain       = domain,
        unix_user    = unix_user,
        php_version  = php_version,
    )
}

pub fn reload_nginx() -> Result<(), Status> {
    validate_nginx_config()?;
    let status = Command::new("/usr/sbin/nginx")
        .args(["-s", "reload"])
        .status()
        .map_err(|e| Status::internal(format!("nginx -s reload exec failed: {}", e)))?;

    if !status.success() {
        return Err(Status::internal("nginx -s reload returned non-zero"));
    }
    info!("nginx reloaded");
    Ok(())
}

fn validate_nginx_config() -> Result<(), Status> {
    let output = Command::new("/usr/sbin/nginx")
        .args(["-t"])
        .output()
        .map_err(|e| Status::internal(format!("nginx -t exec failed: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Status::internal(format!("nginx -t failed: {}", stderr)));
    }
    Ok(())
}

// ── OLS (OpenLiteSpeed) vhost ─────────────────────────────────────────────────

/// Write an OLS vhost config for a site.
pub fn write_ols_vhost(
    domain: &str,
    unix_user: &str,
    admin_email: &str,
) -> Result<(), Status> {
    let vhost_dir = format!("/usr/local/lsws/conf/vhosts/{}", domain);
    std::fs::create_dir_all(&vhost_dir)
        .map_err(|e| Status::internal(format!("create OLS vhost dir failed: {}", e)))?;

    let conf = ols_vhost_template(domain, unix_user, admin_email);
    let conf_path = format!("{}/vhconf.conf", vhost_dir);

    std::fs::write(&conf_path, &conf)
        .map_err(|e| Status::internal(format!("write OLS vhost failed: {}", e)))?;

    info!(%domain, conf_path = %conf_path, "OLS vhost created");
    Ok(())
}

fn ols_vhost_template(domain: &str, unix_user: &str, admin_email: &str) -> String {
    let email = if admin_email.is_empty() {
        format!("admin@{}", domain)
    } else {
        admin_email.to_string()
    };

    format!(
        r#"docRoot                   /home/{unix_user}/public_html
vhDomain                  {domain}
vhAliases                 www.{domain}
adminEmails               {admin_email}
enableGzip                1
enableBr                  1
maxKeepAliveReq           100
keepAliveTimeout          5
enableIpGeo               0

context / {{
  location              /home/{unix_user}/public_html
  allowBrowse           0
  indexFiles            index.php, index.html
  rewrite {{
    enable              1
    autoLoadHtaccess    1
  }}
  addDefaultCharset     off
}}

rewrite {{
  enable                1
  autoLoadHtaccess      1
}}

phpIniOverride {{
  php_admin_value[open_basedir] /home/{unix_user}:/tmp:/usr/share/php
  php_admin_value[upload_max_filesize] 64M
  php_admin_value[post_max_size] 64M
}}

errorlog {{
  useServer             0
  fileName              /var/log/lsws/{domain}.error.log
  logLevel              WARN
}}

accesslog {{
  useServer             0
  fileName              /var/log/lsws/{domain}.access.log
  rotateSize            10M
  keepDays              7
  compress              1
}}
"#,
        domain     = domain,
        unix_user  = unix_user,
        admin_email = email,
    )
}

pub fn reload_ols() -> Result<(), Status> {
    // OLS: try SIGUSR1 via PID file first, fall back to lswsctrl
    let pid_file = "/usr/local/lsws/logs/lshttpd.pid";

    let sent_signal = std::fs::read_to_string(pid_file)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .map(|pid| {
            Command::new("/bin/kill")
                .args(["-SIGUSR1", &pid.to_string()])
                .status()
                .ok()
                .map(|st| st.success())
                .unwrap_or(false)
        })
        .unwrap_or(false);

    if !sent_signal {
        // Fallback: lswsctrl graceful reload
        let status = Command::new("/usr/local/lsws/bin/lswsctrl")
            .arg("reload")
            .status()
            .map_err(|e| Status::internal(format!("lswsctrl reload exec failed: {}", e)))?;

        if !status.success() {
            return Err(Status::internal("lswsctrl reload failed"));
        }
    }

    info!("OLS reloaded");
    Ok(())
}

// ── Apache vhost ──────────────────────────────────────────────────────────────

pub fn write_apache_vhost(
    domain: &str,
    unix_user: &str,
    php_version: &str,
    socket_path: &str,
) -> Result<(), Status> {
    let conf_dir = "/etc/apache2/sites-available";
    std::fs::create_dir_all(conf_dir)
        .map_err(|e| Status::internal(format!("create apache sites-available failed: {}", e)))?;

    let conf      = apache_vhost_template(domain, unix_user, php_version, socket_path);
    let conf_path = format!("{}/{}.conf", conf_dir, domain);

    std::fs::write(&conf_path, &conf)
        .map_err(|e| Status::internal(format!("write apache vhost failed: {}", e)))?;

    // Enable site
    let _ = Command::new("/usr/sbin/a2ensite")
        .arg(&format!("{}.conf", domain))
        .status();

    info!(%domain, "Apache vhost created");
    Ok(())
}

fn apache_vhost_template(
    domain: &str,
    unix_user: &str,
    php_version: &str,
    socket_path: &str,
) -> String {
    format!(
        r#"<VirtualHost *:80>
    ServerName {domain}
    ServerAlias www.{domain}
    DocumentRoot /home/{unix_user}/public_html

    <Directory /home/{unix_user}/public_html>
        Options -Indexes +FollowSymLinks
        AllowOverride All
        Require all granted
        php_admin_value open_basedir "/home/{unix_user}:/tmp:/usr/share/php"
    </Directory>

    # PHP {php_version} via FPM socket
    <FilesMatch \.php$>
        SetHandler "proxy:unix:{socket_path}|fcgi://localhost"
    </FilesMatch>

    ErrorLog  /var/log/apache2/{domain}.error.log
    CustomLog /var/log/apache2/{domain}.access.log combined
</VirtualHost>
"#,
        domain      = domain,
        unix_user   = unix_user,
        php_version = php_version,
        socket_path = socket_path,
    )
}

pub fn reload_apache() -> Result<(), Status> {
    let status = Command::new("/usr/sbin/apachectl")
        .args(["graceful"])
        .status()
        .map_err(|e| Status::internal(format!("apachectl graceful exec failed: {}", e)))?;

    if !status.success() {
        return Err(Status::internal("apachectl graceful failed"));
    }
    info!("Apache reloaded");
    Ok(())
}

// ── ReloadWebServer RPC ───────────────────────────────────────────────────────

pub async fn reload_web_server(
    req: ReloadWebServerRequest,
) -> Result<tonic::Response<StatusResponse>, Status> {
    info!(web_server = %req.web_server, "reload_web_server");

    match req.web_server.as_str() {
        "nginx" => reload_nginx()?,
        "ols" | "lse" => reload_ols()?,
        "apache" => reload_apache()?,
        "" => {
            // Reload all web servers that are running
            let _ = reload_nginx();
            let _ = reload_ols();
            let _ = reload_apache();
        }
        other => {
            return Err(Status::invalid_argument(format!(
                "unknown web_server: {:?}",
                other
            )));
        }
    }

    Ok(tonic::Response::new(StatusResponse {
        success: true,
        message: format!("web server {} reloaded", req.web_server),
    }))
}

// ── Vhost SSL update helpers (called from ssl.rs) ─────────────────────────────

/// Append an SSL server block to an existing nginx vhost file.
///
/// `unix_user` must be the actual system user owning the site (e.g.
/// `orbit_example_com`) — it is used to construct the `root` directive.
/// Previously this was derived from the domain name with a different
/// replacement rule, producing a wrong path.
pub fn append_ssl_to_nginx_vhost(
    domain: &str,
    unix_user: &str,
    cert_path: &str,
    key_path: &str,
) -> Result<(), Status> {
    let vhost_path = format!("/etc/nginx/sites-available/{}.conf", domain);

    if !Path::new(&vhost_path).exists() {
        return Err(Status::not_found(format!(
            "nginx vhost for {} not found",
            domain
        )));
    }

    let mut existing = std::fs::read_to_string(&vhost_path)
        .map_err(|e| Status::internal(format!("read nginx vhost failed: {}", e)))?;

    // Append an SSL server block if not already present
    if !existing.contains("listen 443 ssl") {
        let ssl_block = format!(
            r#"
server {{
    listen 443 ssl http2;
    listen [::]:443 ssl http2;
    server_name {domain} www.{domain};
    root /home/{unix_user}/public_html;

    ssl_certificate     {cert_path};
    ssl_certificate_key {key_path};
    ssl_protocols       TLSv1.2 TLSv1.3;
    ssl_ciphers         ECDHE-ECDSA-AES128-GCM-SHA256:ECDHE-RSA-AES128-GCM-SHA256:ECDHE-ECDSA-AES256-GCM-SHA384:ECDHE-RSA-AES256-GCM-SHA384:ECDHE-ECDSA-CHACHA20-POLY1305:ECDHE-RSA-CHACHA20-POLY1305:DHE-RSA-AES128-GCM-SHA256;
    ssl_prefer_server_ciphers off;
    ssl_session_timeout 1d;
    ssl_session_cache   shared:SSL:10m;
    ssl_stapling        on;
    ssl_stapling_verify on;

    add_header Strict-Transport-Security "max-age=31536000; includeSubDomains; preload" always;
    add_header X-Content-Type-Options nosniff always;
    add_header X-Frame-Options SAMEORIGIN always;

    include /etc/nginx/sites-available/{domain}.conf-location;
}}
"#,
            domain    = domain,
            unix_user = unix_user,
            cert_path = cert_path,
            key_path  = key_path,
        );

        existing.push_str(&ssl_block);

        std::fs::write(&vhost_path, &existing)
            .map_err(|e| Status::internal(format!("write updated nginx vhost failed: {}", e)))?;
    }

    validate_nginx_config()?;
    reload_nginx()?;

    Ok(())
}
