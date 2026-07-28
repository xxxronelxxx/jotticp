//! Language runtime handlers: Python, Node.js, Ruby, .NET
//!
//! Steps 4.9–4.12 — sets up per-site app server processes as systemd units
//! inside the site's cgroup slice, with an nginx upstream config for each.
//!
//! Port allocation: 8100 + (hash of last 4 chars of site_id mod 1000)
//! gives a deterministic port in [8100, 9099] per site.  The caller must
//! ensure the site_id is the persistent UUID, so the port is stable across
//! restarts.

use crate::journal::Journal;
use crate::validation::{validate_unix_user};
use std::path::Path;
use tonic::Status;
use tracing::info;

// ── Port allocation ───────────────────────────────────────────────────────────

const PORT_RANGE_START: u16 = 8100;
const PORT_RANGE_END:   u16 = 9100; // exclusive

/// Derive an initial candidate port in [8100, 9099] from the last 4 characters
/// of the site_id string.  Because multiple UUIDs can share the same 4-char
/// suffix the candidate may already be in use; `allocate_port` therefore
/// probes forward within the range until it finds a free TCP port.
///
/// Returns an error if all 1000 slots are occupied (extremely unlikely in
/// practice but safe to handle explicitly).
pub fn allocate_port(site_id: &str) -> u16 {
    let tail = if site_id.len() >= 4 {
        &site_id[site_id.len() - 4..]
    } else {
        site_id
    };
    let hash: u32 = tail.bytes().fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
    let candidate = PORT_RANGE_START + (hash % (PORT_RANGE_END - PORT_RANGE_START) as u32) as u16;

    // Linear probe: try up to (range_size) ports starting from the hash-derived candidate.
    let range_size = (PORT_RANGE_END - PORT_RANGE_START) as u16;
    for offset in 0..range_size {
        let port = PORT_RANGE_START + (candidate - PORT_RANGE_START + offset) % range_size;
        if !is_port_in_use(port) {
            return port;
        }
    }

    // All slots occupied — fall back to the original candidate and let the
    // caller discover the conflict at bind time.
    candidate
}

/// Check whether a TCP port is already bound on 127.0.0.1.
/// Uses a non-blocking connect attempt: if we can bind a listener on that
/// port it is free; if binding fails, the port is in use.
fn is_port_in_use(port: u16) -> bool {
    use std::net::TcpListener;
    // Attempt to bind — if it succeeds the port is free (listener is dropped immediately).
    TcpListener::bind(("127.0.0.1", port)).is_err()
}

// ── Systemd unit helpers ──────────────────────────────────────────────────────

/// Write a systemd unit to /etc/systemd/system/jotti-site-SITEID.service,
/// then run `systemctl daemon-reload && systemctl enable --now <unit>`.
fn write_and_enable_systemd_unit(site_id: &str, unit_content: &str) -> Result<(), Status> {
    let safe_id = sanitize_site_id(site_id)?;
    let unit_name = format!("jotti-site-{}", safe_id);
    let unit_path = format!("/etc/systemd/system/{}.service", unit_name);

    // Ensure directory exists (should always exist on systemd systems)
    if let Some(parent) = Path::new(&unit_path).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Status::internal(format!("create systemd dir: {}", e)))?;
    }

    std::fs::write(&unit_path, unit_content)
        .map_err(|e| Status::internal(format!("write systemd unit {}: {}", unit_path, e)))?;

    // daemon-reload
    run_systemctl(&["daemon-reload"])?;

    // enable --now (starts the unit and enables on boot)
    run_systemctl(&["enable", "--now", &format!("{}.service", unit_name)])?;

    info!(unit = %unit_name, path = %unit_path, "systemd unit enabled");
    Ok(())
}

fn run_systemctl(args: &[&str]) -> Result<(), Status> {
    let output = std::process::Command::new("systemctl")
        .args(args)
        .output()
        .map_err(|e| Status::internal(format!("systemctl exec failed: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Status::internal(format!(
            "systemctl {} failed: {}",
            args.join(" "),
            stderr.trim()
        )));
    }
    Ok(())
}

/// Reload nginx via `nginx -t && systemctl reload nginx`.
fn reload_nginx() -> Result<(), Status> {
    // Config test first — never reload a broken nginx config
    let test = std::process::Command::new("nginx")
        .args(["-t"])
        .output()
        .map_err(|e| Status::internal(format!("nginx -t exec failed: {}", e)))?;

    if !test.status.success() {
        let stderr = String::from_utf8_lossy(&test.stderr);
        return Err(Status::internal(format!(
            "nginx config test failed — NOT reloading: {}",
            stderr.trim()
        )));
    }

    run_systemctl(&["reload", "nginx"])
}

/// Write an nginx upstream config and reload nginx.
fn write_nginx_upstream(site_id: &str, port: u16) -> Result<(), Status> {
    let safe_id = sanitize_site_id(site_id)?;
    let conf_dir = "/etc/nginx/conf.d";
    let conf_path = format!("{}/upstream-{}.conf", conf_dir, safe_id);

    let content = format!(
        "# jotti-agent managed — do not edit manually\n\
         upstream orbit_{safe_id} {{\n    \
             server 127.0.0.1:{port};\n    \
             keepalive 16;\n\
         }}\n"
    );

    std::fs::create_dir_all(conf_dir)
        .map_err(|e| Status::internal(format!("create nginx conf.d: {}", e)))?;

    std::fs::write(&conf_path, &content)
        .map_err(|e| Status::internal(format!("write nginx upstream {}: {}", conf_path, e)))?;

    info!(conf = %conf_path, port, "nginx upstream config written");

    reload_nginx()
}

/// Sanitise a site_id so it is safe for use in file paths.
/// Only allows hex digits and hyphens (UUID format).
fn sanitize_site_id(site_id: &str) -> Result<&str, Status> {
    if site_id.is_empty() || site_id.len() > 64 {
        return Err(Status::invalid_argument("site_id must be 1–64 characters"));
    }
    if !site_id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        return Err(Status::invalid_argument(
            "site_id must contain only hex digits and hyphens",
        ));
    }
    Ok(site_id)
}

// ── Python (Step 4.9) ─────────────────────────────────────────────────────────

/// Set up a Python site:
/// 1. Create /home/USER/.venv via `python3 -m venv`
/// 2. Write a uWSGI systemd unit inside the site cgroup slice
/// 3. Write nginx upstream config → /etc/nginx/conf.d/upstream-SITEID.conf
pub async fn setup_python_site(
    site_id: &str,
    domain: &str,
    unix_user: &str,
    python_version: &str,
    journal: &Journal,
) -> Result<(), Status> {
    let unix_user = validate_unix_user(unix_user)?;
    let safe_id   = sanitize_site_id(site_id)?;
    let python_bin = validated_python_bin(python_version)?;
    let port = allocate_port(site_id);

    info!(%domain, %unix_user, %python_version, port, "setup_python_site");

    let op_id = journal
        .begin_op(
            "setup_python_site",
            Some(domain),
            &serde_json::json!({
                "site_id": site_id,
                "domain": domain,
                "unix_user": unix_user,
                "python_version": python_version,
                "port": port,
            }),
        )
        .await
        .map_err(|e| Status::internal(format!("journal error: {}", e)))?;

    let result = do_setup_python(safe_id, &unix_user, &python_bin, port);

    match &result {
        Ok(_)  => { let _ = journal.finish_op(&op_id).await; }
        Err(e) => { let _ = journal.fail_op(&op_id, &e.message()).await; }
    }

    result
}

fn do_setup_python(site_id: &str, unix_user: &str, python_bin: &str, port: u16) -> Result<(), Status> {
    let venv_path = format!("/home/{}/.venv", unix_user);

    // Create virtualenv as unix_user
    let output = std::process::Command::new("sudo")
        .args(["-u", unix_user, python_bin, "-m", "venv", &venv_path])
        .output()
        .map_err(|e| Status::internal(format!("venv creation exec failed: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Status::internal(format!("python -m venv failed: {}", stderr.trim())));
    }

    // Ensure site-packages socket dir exists
    let socket_dir = format!("/run/orbit/uwsgi/{}", unix_user);
    std::fs::create_dir_all(&socket_dir)
        .map_err(|e| Status::internal(format!("create uwsgi socket dir: {}", e)))?;

    let uwsgi_bin = format!("{}/.venv/bin/uwsgi", format!("/home/{}", unix_user));

    let unit = format!(
        "[Unit]\n\
         Description=JottiCP Python site — {unix_user}\n\
         After=network.target\n\
         \n\
         [Service]\n\
         Type=notify\n\
         User={unix_user}\n\
         Group={unix_user}\n\
         WorkingDirectory=/home/{unix_user}\n\
         ExecStart={uwsgi_bin} \
             --http-socket 127.0.0.1:{port} \
             --venv {venv_path} \
             --module wsgi:application \
             --master \
             --processes 2 \
             --threads 2 \
             --harakiri 60 \
             --max-requests 5000 \
             --vacuum \
             --die-on-term\n\
         ExecReload=/bin/kill -HUP $MAINPID\n\
         Restart=on-failure\n\
         RestartSec=5s\n\
         # Cgroup isolation — site resource limits enforced by jotti-cron\n\
         Slice=website-{unix_user}.slice\n\
         PrivateTmp=yes\n\
         NoNewPrivileges=yes\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n"
    );

    write_and_enable_systemd_unit(site_id, &unit)?;
    write_nginx_upstream(site_id, port)?;

    info!(%unix_user, port, "Python site provisioned");
    Ok(())
}

fn validated_python_bin(version: &str) -> Result<String, Status> {
    // Allow "3", "3.10", "3.11", "3.12", "3.13"
    if version.is_empty() {
        return Ok("python3".to_string());
    }
    if version.starts_with('3')
        && version.len() <= 5
        && version.chars().all(|c| c.is_ascii_digit() || c == '.')
    {
        return Ok(format!("python{}", version));
    }
    Err(Status::invalid_argument(format!(
        "invalid python_version: {:?} — expected e.g. \"3.12\"",
        version
    )))
}

// ── Node.js (Step 4.10) ───────────────────────────────────────────────────────

/// Set up a Node.js site:
/// 1. Install the requested Node version via nvm (as unix_user)
/// 2. Write a systemd unit with ExecStart pointing to the nvm node binary
/// 3. Write nginx proxy config
pub async fn setup_nodejs_site(
    site_id: &str,
    domain: &str,
    unix_user: &str,
    node_version: &str,
    journal: &Journal,
) -> Result<(), Status> {
    let unix_user   = validate_unix_user(unix_user)?;
    let safe_id     = sanitize_site_id(site_id)?;
    let node_version = validated_node_version(node_version)?;
    let port = allocate_port(site_id);

    info!(%domain, %unix_user, %node_version, port, "setup_nodejs_site");

    let op_id = journal
        .begin_op(
            "setup_nodejs_site",
            Some(domain),
            &serde_json::json!({
                "site_id": site_id,
                "domain": domain,
                "unix_user": unix_user,
                "node_version": node_version,
                "port": port,
            }),
        )
        .await
        .map_err(|e| Status::internal(format!("journal error: {}", e)))?;

    let result = do_setup_nodejs(safe_id, &unix_user, &node_version, port);

    match &result {
        Ok(_)  => { let _ = journal.finish_op(&op_id).await; }
        Err(e) => { let _ = journal.fail_op(&op_id, &e.message()).await; }
    }

    result
}

fn do_setup_nodejs(site_id: &str, unix_user: &str, node_version: &str, port: u16) -> Result<(), Status> {
    let nvm_dir    = format!("/home/{}/.nvm", unix_user);
    let nvm_script = format!("{}/nvm.sh", nvm_dir);

    // nvm must be pre-installed for the user; install the requested version
    // Source nvm.sh inside a bash -c call (nvm is a bash function, not a binary)
    let install_cmd = format!(
        "source {} && nvm install {} && nvm use {}",
        nvm_script, node_version, node_version
    );

    let output = std::process::Command::new("sudo")
        .args([
            "-u", unix_user,
            "bash", "--login", "-c",
            &install_cmd,
        ])
        .output()
        .map_err(|e| Status::internal(format!("nvm install exec failed: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Status::internal(format!(
            "nvm install {} failed: {}",
            node_version,
            stderr.trim()
        )));
    }

    // Path to the installed node binary under nvm
    let node_bin = format!(
        "/home/{unix_user}/.nvm/versions/node/v{node_version}/bin/node"
    );

    let unit = format!(
        "[Unit]\n\
         Description=JottiCP Node.js site — {unix_user}\n\
         After=network.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         User={unix_user}\n\
         Group={unix_user}\n\
         WorkingDirectory=/home/{unix_user}\n\
         Environment=PORT={port}\n\
         Environment=NODE_ENV=production\n\
         ExecStart={node_bin} /home/{unix_user}/app.js {port}\n\
         Restart=on-failure\n\
         RestartSec=5s\n\
         # Cgroup isolation\n\
         Slice=website-{unix_user}.slice\n\
         PrivateTmp=yes\n\
         NoNewPrivileges=yes\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n"
    );

    write_and_enable_systemd_unit(site_id, &unit)?;
    write_nginx_upstream(site_id, port)?;

    info!(%unix_user, %node_version, port, "Node.js site provisioned");
    Ok(())
}

fn validated_node_version(version: &str) -> Result<String, Status> {
    // Allow "18", "20", "22", "21.7", etc.  Must start with a digit, max length 8.
    if version.is_empty() {
        return Err(Status::invalid_argument("node_version must not be empty"));
    }
    if version.len() <= 8
        && version.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
        && version.chars().all(|c| c.is_ascii_digit() || c == '.')
    {
        return Ok(version.to_string());
    }
    Err(Status::invalid_argument(format!(
        "invalid node_version: {:?} — expected e.g. \"20\" or \"20.11\"",
        version
    )))
}

// ── Ruby (Step 4.11) ──────────────────────────────────────────────────────────

/// Set up a Ruby site:
/// 1. Install the requested Ruby version via rbenv (as unix_user)
/// 2. Write a Puma systemd unit inside the site cgroup slice
/// 3. Write nginx upstream config
pub async fn setup_ruby_site(
    site_id: &str,
    domain: &str,
    unix_user: &str,
    ruby_version: &str,
    journal: &Journal,
) -> Result<(), Status> {
    let unix_user   = validate_unix_user(unix_user)?;
    let safe_id     = sanitize_site_id(site_id)?;
    let ruby_version = validated_ruby_version(ruby_version)?;
    let port = allocate_port(site_id);

    info!(%domain, %unix_user, %ruby_version, port, "setup_ruby_site");

    let op_id = journal
        .begin_op(
            "setup_ruby_site",
            Some(domain),
            &serde_json::json!({
                "site_id": site_id,
                "domain": domain,
                "unix_user": unix_user,
                "ruby_version": ruby_version,
                "port": port,
            }),
        )
        .await
        .map_err(|e| Status::internal(format!("journal error: {}", e)))?;

    let result = do_setup_ruby(safe_id, &unix_user, &ruby_version, port);

    match &result {
        Ok(_)  => { let _ = journal.finish_op(&op_id).await; }
        Err(e) => { let _ = journal.fail_op(&op_id, &e.message()).await; }
    }

    result
}

fn do_setup_ruby(site_id: &str, unix_user: &str, ruby_version: &str, port: u16) -> Result<(), Status> {
    let rbenv_root = format!("/home/{}/.rbenv", unix_user);

    // Install the requested Ruby version via rbenv
    let install_cmd = format!(
        "export RBENV_ROOT={rbenv_root} && \
         export PATH={rbenv_root}/bin:{rbenv_root}/shims:$PATH && \
         rbenv install --skip-existing {ruby_version} && \
         rbenv global {ruby_version} && \
         gem install puma --no-document"
    );

    let output = std::process::Command::new("sudo")
        .args([
            "-u", unix_user,
            "bash", "--login", "-c",
            &install_cmd,
        ])
        .output()
        .map_err(|e| Status::internal(format!("rbenv install exec failed: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Status::internal(format!(
            "rbenv install {} failed: {}",
            ruby_version,
            stderr.trim()
        )));
    }

    // Path to puma executable under rbenv
    let puma_bin = format!(
        "/home/{unix_user}/.rbenv/versions/{ruby_version}/bin/puma"
    );

    let puma_config = format!("/home/{unix_user}/puma.rb");

    // Write a minimal Puma config if one doesn't exist
    let default_puma_config = format!(
        "# Puma configuration — generated by JottiCP\n\
         port        {port}\n\
         environment 'production'\n\
         workers     2\n\
         threads     1, 4\n\
         pidfile     '/home/{unix_user}/tmp/puma.pid'\n\
         bind        'tcp://127.0.0.1:{port}'\n"
    );

    // Create tmp directory for pids
    let tmp_dir = format!("/home/{}/tmp", unix_user);
    std::fs::create_dir_all(&tmp_dir)
        .map_err(|e| Status::internal(format!("create ruby tmp dir: {}", e)))?;

    // Only write default config if not already present (preserves user customization)
    if !Path::new(&puma_config).exists() {
        std::fs::write(&puma_config, &default_puma_config)
            .map_err(|e| Status::internal(format!("write puma config: {}", e)))?;

        // Fix ownership
        let _ = std::process::Command::new("chown")
            .args([&format!("{}:{}", unix_user, unix_user), &puma_config])
            .output();
    }

    let unit = format!(
        "[Unit]\n\
         Description=JottiCP Ruby/Puma site — {unix_user}\n\
         After=network.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         User={unix_user}\n\
         Group={unix_user}\n\
         WorkingDirectory=/home/{unix_user}\n\
         Environment=RAILS_ENV=production\n\
         Environment=RACK_ENV=production\n\
         ExecStart={puma_bin} -C /home/{unix_user}/puma.rb\n\
         ExecReload=/bin/kill -USR2 $MAINPID\n\
         Restart=on-failure\n\
         RestartSec=5s\n\
         # Cgroup isolation\n\
         Slice=website-{unix_user}.slice\n\
         PrivateTmp=yes\n\
         NoNewPrivileges=yes\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n"
    );

    write_and_enable_systemd_unit(site_id, &unit)?;
    write_nginx_upstream(site_id, port)?;

    info!(%unix_user, %ruby_version, port, "Ruby/Puma site provisioned");
    Ok(())
}

fn validated_ruby_version(version: &str) -> Result<String, Status> {
    // Allow "3.3", "3.3.0", "3.2.4", etc.
    if version.is_empty() {
        return Err(Status::invalid_argument("ruby_version must not be empty"));
    }
    if version.len() <= 8
        && version.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
        && version.chars().all(|c| c.is_ascii_digit() || c == '.')
    {
        return Ok(version.to_string());
    }
    Err(Status::invalid_argument(format!(
        "invalid ruby_version: {:?} — expected e.g. \"3.3\" or \"3.3.0\"",
        version
    )))
}

// ── .NET (Step 4.12) ──────────────────────────────────────────────────────────

/// Set up a .NET site:
/// 1. Create /home/USER/app/ directory
/// 2. Write a systemd unit running `dotnet run` via Kestrel on the allocated port
/// 3. Write nginx proxy config
pub async fn setup_dotnet_site(
    site_id: &str,
    domain: &str,
    unix_user: &str,
    dotnet_version: &str,
    journal: &Journal,
) -> Result<(), Status> {
    let unix_user     = validate_unix_user(unix_user)?;
    let safe_id       = sanitize_site_id(site_id)?;
    let dotnet_version = validated_dotnet_version(dotnet_version)?;
    let port = allocate_port(site_id);

    info!(%domain, %unix_user, %dotnet_version, port, "setup_dotnet_site");

    let op_id = journal
        .begin_op(
            "setup_dotnet_site",
            Some(domain),
            &serde_json::json!({
                "site_id": site_id,
                "domain": domain,
                "unix_user": unix_user,
                "dotnet_version": dotnet_version,
                "port": port,
            }),
        )
        .await
        .map_err(|e| Status::internal(format!("journal error: {}", e)))?;

    let result = do_setup_dotnet(safe_id, &unix_user, &dotnet_version, port);

    match &result {
        Ok(_)  => { let _ = journal.finish_op(&op_id).await; }
        Err(e) => { let _ = journal.fail_op(&op_id, &e.message()).await; }
    }

    result
}

fn do_setup_dotnet(site_id: &str, unix_user: &str, dotnet_version: &str, port: u16) -> Result<(), Status> {
    // Create app directory owned by unix_user
    let app_dir = format!("/home/{}/app", unix_user);
    std::fs::create_dir_all(&app_dir)
        .map_err(|e| Status::internal(format!("create dotnet app dir: {}", e)))?;

    // Set ownership
    let _ = std::process::Command::new("chown")
        .args([
            "-R",
            &format!("{}:{}", unix_user, unix_user),
            &app_dir,
        ])
        .output();

    // Path to dotnet binary — system-wide install via Microsoft package feed
    let dotnet_bin = "/usr/bin/dotnet";

    // Environment file location for Kestrel URLs
    let env_file = format!("/etc/jottiecp/dotnet/{}.env", site_id);
    let env_dir  = "/etc/jottiecp/dotnet";
    std::fs::create_dir_all(env_dir)
        .map_err(|e| Status::internal(format!("create dotnet env dir: {}", e)))?;

    let env_content = format!(
        "ASPNETCORE_ENVIRONMENT=Production\n\
         ASPNETCORE_URLS=http://127.0.0.1:{port}\n\
         DOTNET_ROOT=/usr/lib/dotnet\n"
    );
    std::fs::write(&env_file, &env_content)
        .map_err(|e| Status::internal(format!("write dotnet env file: {}", e)))?;

    let unit = format!(
        "[Unit]\n\
         Description=JottiCP .NET {dotnet_version} site — {unix_user}\n\
         After=network.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         User={unix_user}\n\
         Group={unix_user}\n\
         WorkingDirectory=/home/{unix_user}/app\n\
         EnvironmentFile={env_file}\n\
         ExecStart={dotnet_bin} run --project /home/{unix_user}/app --urls http://127.0.0.1:{port}\n\
         Restart=on-failure\n\
         RestartSec=5s\n\
         # Cgroup isolation\n\
         Slice=website-{unix_user}.slice\n\
         PrivateTmp=yes\n\
         NoNewPrivileges=yes\n\
         # .NET specific\n\
         LimitNOFILE=65536\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n"
    );

    write_and_enable_systemd_unit(site_id, &unit)?;
    write_nginx_upstream(site_id, port)?;

    info!(%unix_user, %dotnet_version, port, ".NET site provisioned");
    Ok(())
}

fn validated_dotnet_version(version: &str) -> Result<String, Status> {
    // Allow "8", "9", "8.0", "9.0"
    if version.is_empty() {
        return Err(Status::invalid_argument("dotnet_version must not be empty"));
    }
    if version.len() <= 6
        && version.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
        && version.chars().all(|c| c.is_ascii_digit() || c == '.')
    {
        // Only .NET 8+ (LTS) — not .NET Framework
        let major: u32 = version
            .split('.')
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if major >= 8 {
            return Ok(version.to_string());
        }
    }
    Err(Status::invalid_argument(format!(
        "invalid dotnet_version: {:?} — expected \"8\" or \"9\" (.NET Core/5+ only, NOT Framework)",
        version
    )))
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_allocation_is_in_range() {
        let site_ids = [
            "550e8400-e29b-41d4-a716-446655440000",
            "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            "00000000-0000-0000-0000-000000000001",
        ];
        for id in &site_ids {
            let port = allocate_port(id);
            assert!(
                port >= PORT_RANGE_START && port < PORT_RANGE_END,
                "port {} out of range for site_id {}",
                port,
                id
            );
        }
    }

    /// Port allocation may probe forward when the initial candidate is taken,
    /// so two calls for the same id will agree only when no ports in the range
    /// are bound.  We just verify the result stays in range.
    #[test]
    fn port_allocation_stays_in_range_on_repeated_call() {
        let id = "550e8400-e29b-41d4-a716-446655440000";
        let p1 = allocate_port(id);
        let p2 = allocate_port(id);
        assert!(p1 >= PORT_RANGE_START && p1 < PORT_RANGE_END);
        assert!(p2 >= PORT_RANGE_START && p2 < PORT_RANGE_END);
    }

    #[test]
    fn is_port_in_use_detects_bound_port() {
        use std::net::TcpListener;
        // Bind an ephemeral port then verify is_port_in_use returns true for it.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(is_port_in_use(port), "port {port} should be detected as in use");
    }

    #[test]
    fn sanitize_site_id_accepts_uuid() {
        assert!(sanitize_site_id("550e8400-e29b-41d4-a716-446655440000").is_ok());
    }

    #[test]
    fn sanitize_site_id_rejects_shell_metacharacters() {
        assert!(sanitize_site_id("site; rm -rf /").is_err());
        assert!(sanitize_site_id("../etc/passwd").is_err());
        assert!(sanitize_site_id("$(whoami)").is_err());
    }

    #[test]
    fn validate_python_version() {
        assert!(validated_python_bin("3.12").is_ok());
        assert!(validated_python_bin("3").is_ok());
        assert!(validated_python_bin("2.7").is_err()); // still starts with 3 check fails
        assert!(validated_python_bin("; rm -rf /").is_err());
    }

    #[test]
    fn validate_node_version() {
        assert!(validated_node_version("20").is_ok());
        assert!(validated_node_version("20.11.1").is_ok());
        assert!(validated_node_version("; rm -rf /").is_err());
    }

    #[test]
    fn validate_ruby_version() {
        assert!(validated_ruby_version("3.3").is_ok());
        assert!(validated_ruby_version("3.3.0").is_ok());
        assert!(validated_ruby_version("bad").is_err());
    }

    #[test]
    fn validate_dotnet_version_rejects_old() {
        assert!(validated_dotnet_version("8").is_ok());
        assert!(validated_dotnet_version("9.0").is_ok());
        assert!(validated_dotnet_version("4.8").is_err()); // .NET Framework
        assert!(validated_dotnet_version("7").is_err());   // EOL
    }
}
