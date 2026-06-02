//! File operation handlers: ExtractArchive, RunCommand.

use crate::proto::{ExtractArchiveRequest, RunCommandRequest, RunCommandResponse, StatusResponse};
use crate::validation::{validate_safe_path, validate_unix_user};
use std::io::Read;
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use tonic::Status;
use tracing::{info, warn};

// Maximum bytes returned in stdout/stderr (4KB each)
const MAX_OUTPUT_BYTES: usize = 4096;

// Default command timeout
const DEFAULT_TIMEOUT_SECS: u64 = 60;

// ── ExtractArchive ────────────────────────────────────────────────────────────

pub async fn extract_archive(
    req: ExtractArchiveRequest,
) -> Result<tonic::Response<StatusResponse>, Status> {
    let archive_path = validate_safe_path(&req.archive_path)?;
    let dest_path    = validate_safe_path(&req.dest_path)?;
    let unix_user    = validate_unix_user(&req.unix_user)?;

    info!(
        archive = %archive_path,
        dest    = %dest_path,
        user    = %unix_user,
        "extract_archive"
    );

    if !Path::new(&archive_path).exists() {
        return Err(Status::not_found(format!(
            "archive not found: {}",
            archive_path
        )));
    }

    // Create destination directory
    std::fs::create_dir_all(&dest_path)
        .map_err(|e| Status::internal(format!("create dest dir failed: {}", e)))?;

    // Detect archive type by extension
    if archive_path.ends_with(".tar.gz") || archive_path.ends_with(".tgz") {
        extract_targz(&archive_path, &dest_path)?;
    } else if archive_path.ends_with(".tar.bz2") || archive_path.ends_with(".tbz2") {
        extract_tarbz2(&archive_path, &dest_path)?;
    } else if archive_path.ends_with(".zip") {
        extract_zip(&archive_path, &dest_path)?;
    } else {
        return Err(Status::invalid_argument(format!(
            "unsupported archive format: {}",
            archive_path
        )));
    }

    // chown extracted files to the site user
    let status = Command::new("/bin/chown")
        .args(["-R", &format!("{}:{}", unix_user, unix_user), &dest_path])
        .status()
        .map_err(|e| Status::internal(format!("chown exec failed: {}", e)))?;

    if !status.success() {
        warn!(dest = %dest_path, user = %unix_user, "chown after extract returned non-zero");
    }

    info!(archive = %archive_path, dest = %dest_path, "archive extracted");

    Ok(tonic::Response::new(StatusResponse {
        success: true,
        message: format!("extracted {} to {}", archive_path, dest_path),
    }))
}

fn extract_targz(archive: &str, dest: &str) -> Result<(), Status> {
    let status = Command::new("/bin/tar")
        .args(["-xzf", archive, "-C", dest])
        .status()
        .map_err(|e| Status::internal(format!("tar -xzf exec failed: {}", e)))?;

    if !status.success() {
        return Err(Status::internal(format!(
            "tar -xzf {} failed with exit code {:?}",
            archive,
            status.code()
        )));
    }
    Ok(())
}

fn extract_tarbz2(archive: &str, dest: &str) -> Result<(), Status> {
    let status = Command::new("/bin/tar")
        .args(["-xjf", archive, "-C", dest])
        .status()
        .map_err(|e| Status::internal(format!("tar -xjf exec failed: {}", e)))?;

    if !status.success() {
        return Err(Status::internal(format!(
            "tar -xjf {} failed with exit code {:?}",
            archive,
            status.code()
        )));
    }
    Ok(())
}

fn extract_zip(archive: &str, dest: &str) -> Result<(), Status> {
    let status = Command::new("/usr/bin/unzip")
        .args(["-o", archive, "-d", dest])
        .status()
        .map_err(|e| Status::internal(format!("unzip exec failed: {}", e)))?;

    if !status.success() {
        return Err(Status::internal(format!(
            "unzip {} failed with exit code {:?}",
            archive,
            status.code()
        )));
    }
    Ok(())
}

// ── RunCommand ────────────────────────────────────────────────────────────────

pub async fn run_command(
    req: RunCommandRequest,
) -> Result<tonic::Response<RunCommandResponse>, Status> {
    let unix_user   = validate_unix_user(&req.unix_user)?;
    let working_dir = if req.working_dir.is_empty() {
        format!("/home/{}", unix_user)
    } else {
        validate_safe_path(&req.working_dir)?
    };

    // The command must be an absolute path to a binary — never a shell command
    if !req.command.starts_with('/') {
        return Err(Status::invalid_argument(
            "command must be an absolute path to a binary",
        ));
    }
    let command_path = validate_safe_path(&req.command)?;

    // Reject shell metacharacters in command path
    if command_path.contains(';')
        || command_path.contains('&')
        || command_path.contains('|')
        || command_path.contains('`')
        || command_path.contains('$')
    {
        return Err(Status::invalid_argument(
            "command path contains invalid characters",
        ));
    }

    let timeout_secs = if req.timeout_sec == 0 {
        DEFAULT_TIMEOUT_SECS
    } else {
        req.timeout_sec as u64
    };

    info!(
        user    = %unix_user,
        command = %command_path,
        args    = ?req.args,
        wd      = %working_dir,
        timeout = timeout_secs,
        "run_command"
    );

    // Build command as site user via sudo (no shell, direct exec)
    // sudo is used to drop privileges to the site user safely.
    let mut cmd = Command::new("/usr/bin/sudo");
    cmd.args(["-u", &unix_user, "--", &command_path]);
    for arg in &req.args {
        cmd.arg(arg);
    }
    cmd.current_dir(&working_dir);
    // Clear environment — only set minimal safe vars
    cmd.env_clear();
    cmd.env("HOME", format!("/home/{}", unix_user));
    cmd.env("USER", &unix_user);
    cmd.env("PATH", "/usr/local/bin:/usr/bin:/bin");

    // Run with timeout using tokio
    let output = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        tokio::task::spawn_blocking(move || cmd.output()),
    )
    .await
    .map_err(|_| {
        Status::deadline_exceeded(format!("command timed out after {}s", timeout_secs))
    })?
    .map_err(|e| Status::internal(format!("spawn_blocking failed: {}", e)))?
    .map_err(|e| Status::internal(format!("command exec failed: {}", e)))?;

    let exit_code = output.status.code().unwrap_or(-1);
    let success   = output.status.success();

    // Truncate output to MAX_OUTPUT_BYTES
    let stdout = truncate_utf8(&output.stdout, MAX_OUTPUT_BYTES);
    let stderr = truncate_utf8(&output.stderr, MAX_OUTPUT_BYTES);

    if !success {
        warn!(
            command   = %command_path,
            exit_code = %exit_code,
            stderr    = %stderr,
            "run_command returned non-zero"
        );
    }

    Ok(tonic::Response::new(RunCommandResponse {
        success,
        exit_code,
        stdout,
        stderr,
    }))
}

fn truncate_utf8(bytes: &[u8], max_bytes: usize) -> String {
    if bytes.len() <= max_bytes {
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        // Truncate at a valid UTF-8 boundary
        let truncated = &bytes[..max_bytes];
        let s = String::from_utf8_lossy(truncated).into_owned();
        format!("{}\n[... truncated at {} bytes]", s, max_bytes)
    }
}
