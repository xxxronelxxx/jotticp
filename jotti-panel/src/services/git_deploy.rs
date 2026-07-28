use crate::AppState;
use anyhow::{Context, Result};
use std::sync::Arc;
use uuid::Uuid;

// Bare-repo root on the server filesystem
const GIT_ROOT: &str = "/var/jottiecp/git";

// ── Public API ────────────────────────────────────────────────────────────────

/// Enable git push-to-deploy for a site.
///
/// Creates `/var/jottiecp/git/SITEID.git` (bare repo) and installs a post-receive
/// hook that checks out the pushed ref into the site's home directory.
///
/// Returns the git remote URL the user should add, e.g. `git@HOST:SITEID`.
pub async fn enable_git_deploy(state: &Arc<AppState>, site_id: Uuid) -> Result<String> {
    let site = fetch_site(state, site_id).await?;
    let repo_path = repo_path_for(site_id);

    // ── 1. Initialise bare repository ────────────────────────────────────────
    if !std::path::Path::new(&repo_path).exists() {
        run_cmd("git", &["init", "--bare", &repo_path])
            .context("git init --bare failed")?;
    }

    // Ensure the jottiecp user owns the bare repo
    run_cmd("chown", &["-R", "jottiecp:jottiecp", &repo_path])
        .context("chown git repo failed")?;

    // ── 2. Write post-receive hook ────────────────────────────────────────────
    let hook_path = format!("{}/hooks/post-receive", repo_path);
    let hook = build_hook(&site.id.to_string(), &site.unix_user, &site.domain);
    std::fs::write(&hook_path, hook)
        .with_context(|| format!("Failed to write hook at {}", hook_path))?;
    set_executable(&hook_path)?;

    // ── 3. Persist to DB ──────────────────────────────────────────────────────
    sqlx::query!(
        r#"UPDATE sites
           SET git_deploy_enabled = true,
               git_repo_path      = $2
           WHERE id = $1"#,
        site_id,
        repo_path,
    )
    .execute(&state.db)
    .await
    .context("Failed to update site git_deploy_enabled")?;

    // ── 4. Build remote URL ───────────────────────────────────────────────────
    let server_hostname = hostname();
    let remote_url = format!("git@{}:{}", server_hostname, site_id);
    Ok(remote_url)
}

/// Disable git push-to-deploy: removes bare repo and clears DB columns.
pub async fn disable_git_deploy(state: &Arc<AppState>, site_id: Uuid) -> Result<()> {
    let repo_path = repo_path_for(site_id);

    if std::path::Path::new(&repo_path).exists() {
        std::fs::remove_dir_all(&repo_path)
            .with_context(|| format!("Failed to remove bare repo at {}", repo_path))?;
    }

    sqlx::query!(
        r#"UPDATE sites
           SET git_deploy_enabled = false,
               git_repo_path      = NULL
           WHERE id = $1"#,
        site_id,
    )
    .execute(&state.db)
    .await
    .context("Failed to clear site git_deploy columns")?;

    Ok(())
}

/// Return the last N deployment records for a site (from DB).
pub async fn deploy_history(
    state: &Arc<AppState>,
    site_id: Uuid,
    limit: i64,
) -> Result<Vec<DeployRecord>> {
    let rows = sqlx::query_as!(
        DeployRecord,
        r#"SELECT id, site_id, commit_hash, commit_msg, deployed_at, deployed_by
           FROM git_deploys
           WHERE site_id = $1
           ORDER BY deployed_at DESC
           LIMIT $2"#,
        site_id,
        limit,
    )
    .fetch_all(&state.db)
    .await?;
    Ok(rows)
}

/// Record an incoming deployment (called from the git post-receive hook via
/// the internal `POST /api/v1/sites/:id/git-deploy/record` endpoint).
pub async fn record_deploy(
    state: &Arc<AppState>,
    site_id: Uuid,
    commit_hash: &str,
    commit_msg: Option<&str>,
) -> Result<Uuid> {
    let id = sqlx::query_scalar!(
        r#"INSERT INTO git_deploys (site_id, commit_hash, commit_msg)
           VALUES ($1, $2, $3)
           RETURNING id"#,
        site_id,
        commit_hash,
        commit_msg,
    )
    .fetch_one(&state.db)
    .await?;
    Ok(id)
}

/// Trigger a deployment manually (replays the latest ref of main/master).
pub async fn trigger_deploy(state: &Arc<AppState>, site_id: Uuid) -> Result<String> {
    let site = fetch_site(state, site_id).await?;
    let repo_path = repo_path_for(site_id);

    if !site.git_deploy_enabled {
        anyhow::bail!("Git deploy is not enabled for site {}", site_id);
    }

    // Get the current HEAD of main or master
    let head = run_cmd_output(
        "git",
        &[
            "--git-dir", &repo_path,
            "rev-parse", "--verify", "refs/heads/main",
        ],
    )
    .or_else(|_| {
        run_cmd_output(
            "git",
            &[
                "--git-dir", &repo_path,
                "rev-parse", "--verify", "refs/heads/master",
            ],
        )
    })
    .context("No main or master branch found in bare repo")?;

    let hash = head.trim().to_string();

    // Run checkout
    run_cmd(
        "git",
        &[
            "--git-dir", &repo_path,
            "--work-tree", &format!("/home/{}", site.unix_user),
            "checkout", "-f", &hash,
        ],
    )
    .context("git checkout failed")?;

    // Fix ownership
    run_cmd(
        "chown",
        &["-R", &format!("{}:{}", site.unix_user, site.unix_user),
          &format!("/home/{}", site.unix_user)],
    ).ok();

    // Record
    record_deploy(state, site_id, &hash, Some("manual trigger")).await?;

    Ok(hash)
}

// ── Types ─────────────────────────────────────────────────────────────────────

pub struct DeployRecord {
    pub id:          Uuid,
    pub site_id:     Uuid,
    pub commit_hash: String,
    pub commit_msg:  Option<String>,
    pub deployed_at: chrono::DateTime<chrono::Utc>,
    pub deployed_by: String,
}

struct SiteRow {
    id:                 Uuid,
    domain:             String,
    unix_user:          String,
    git_deploy_enabled: bool,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn repo_path_for(site_id: Uuid) -> String {
    format!("{}/{}.git", GIT_ROOT, site_id)
}

async fn fetch_site(state: &Arc<AppState>, site_id: Uuid) -> Result<SiteRow> {
    sqlx::query_as!(
        SiteRow,
        r#"SELECT id, domain, unix_user, git_deploy_enabled
           FROM sites
           WHERE id = $1 AND deleted_at IS NULL"#,
        site_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| anyhow::anyhow!("Site {} not found", site_id))
}

fn run_cmd(prog: &str, args: &[&str]) -> Result<()> {
    let status = std::process::Command::new(prog)
        .args(args)
        .status()
        .with_context(|| format!("Failed to run {} {:?}", prog, args))?;
    if !status.success() {
        anyhow::bail!("{} exited with status {}", prog, status);
    }
    Ok(())
}

fn run_cmd_output(prog: &str, args: &[&str]) -> Result<String> {
    let out = std::process::Command::new(prog)
        .args(args)
        .output()
        .with_context(|| format!("Failed to run {} {:?}", prog, args))?;
    if !out.status.success() {
        anyhow::bail!(
            "{} failed: {}",
            prog,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn set_executable(path: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "localhost".into())
}

/// Build the post-receive hook script for a site.
fn build_hook(site_id: &str, unix_user: &str, domain: &str) -> String {
    format!(
        r#"#!/bin/bash
# JottiCP post-receive deploy hook for site {site_id} ({domain})
# Generated by jotti-panel — do not edit manually.

SITE_ID="{site_id}"
UNIX_USER="{unix_user}"
SITE_HOME="/home/{unix_user}"
PANEL_URL="http://127.0.0.1:2087"
GIT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

while read oldrev newrev refname; do
    if [ "$refname" = "refs/heads/main" ] || [ "$refname" = "refs/heads/master" ]; then
        echo "JottiCP: Deploying ${{newrev:0:8}} to $SITE_HOME..."

        # Export the new tree to the site home directory
        GIT_WORK_TREE="$SITE_HOME" git --git-dir="$GIT_DIR" checkout -f "$newrev"
        chown -R "$UNIX_USER:$UNIX_USER" "$SITE_HOME"

        # Run composer install if composer.json changed
        if git --git-dir="$GIT_DIR" diff --name-only "$oldrev" "$newrev" 2>/dev/null \
               | grep -q "composer.json"; then
            echo "JottiCP: Running composer install..."
            sudo -u "$UNIX_USER" bash -c "cd '$SITE_HOME' && composer install --no-dev --optimize-autoloader" \
                || echo "JottiCP: composer install failed (non-fatal)"
        fi

        # Run npm ci if package.json changed
        if git --git-dir="$GIT_DIR" diff --name-only "$oldrev" "$newrev" 2>/dev/null \
               | grep -q "^package\.json$"; then
            echo "JottiCP: Running npm ci..."
            sudo -u "$UNIX_USER" bash -c "cd '$SITE_HOME' && npm ci" \
                || echo "JottiCP: npm ci failed (non-fatal)"
        fi

        # Reload PHP-FPM for this site (best-effort)
        systemctl reload "php*-fpm" 2>/dev/null || true

        # Fetch commit message for the deploy record
        COMMIT_MSG=$(git --git-dir="$GIT_DIR" log -1 --pretty=format:"%s" "$newrev" 2>/dev/null || true)

        # Record the deployment in jotti-panel
        INTERNAL_TOKEN="${{ORBIT_INTERNAL_TOKEN:-}}"
        if [ -n "$INTERNAL_TOKEN" ]; then
            curl -s -X POST "$PANEL_URL/api/v1/sites/$SITE_ID/git-deploy/record" \
                -H "Content-Type: application/json" \
                -H "Authorization: Bearer $INTERNAL_TOKEN" \
                -d "{{\"hash\":\"$newrev\",\"commit_msg\":\"$COMMIT_MSG\",\"deployed_at\":\"$(date -Iseconds)\"}}" \
                > /dev/null 2>&1 || true
        fi

        echo "JottiCP: Deploy of ${{newrev:0:8}} complete!"
    fi
done
"#,
        site_id   = site_id,
        unix_user = unix_user,
        domain    = domain,
    )
}
