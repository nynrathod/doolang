//! Process Helpers — Secure command execution and argument parsing.
//!
//! Single source of truth for running commands and parsing args.
//! Cross-platform: handles Windows cmd.exe shell built-ins safely.
//!
//! Security measures:
//! - Windows shell injection prevention via argument sanitization
//! - Environment variable leak prevention (strips secrets)
//! - Command validation (path traversal, null bytes, etc.)
//! - Docker argument sanitization for cloud environments
//! - Output size limits to prevent OOM
//! - Input size limits to prevent DoS

use crate::handle::ProcessHandle;
use crate::registry::get_registry;
use crate::security;

use doo_ffi_core::ffi_debug;
use std::sync::Arc;
use tokio::process::Command;

// ============================================================================
// Cross-platform command builder — Single Source of Truth
// ============================================================================

/// Build a cross-platform Command with security hardening.
///
/// On Windows: routes through cmd.exe /c for shell built-in compatibility,
///             with full argument sanitization against injection.
/// On Linux/Mac: executes directly via execvp (no shell, safe by default).
///
/// Security:
/// - Validates command string (no path traversal, null bytes, etc.)
/// - Sanitizes arguments on Windows (rejects shell metacharacters)
/// - Validates argument count and size
/// - Strips sensitive environment variables from child process
/// - Docker-specific argument validation in cloud mode
fn build_command(cmd: &str, args: &[String]) -> Result<Command, String> {
    // 1. Validate command
    security::validate_command(cmd)?;

    // 2. Docker-specific validation
    let cmd_basename = std::path::Path::new(cmd)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(cmd);
    if cmd_basename == "docker" {
        security::validate_docker_args(args)?;
    }

    // 3. Build command (platform-specific)
    let mut command = if cfg!(windows) {
        // Validate args for Windows shell safety
        if args.len() > security::MAX_ARGS_COUNT {
            return Err(format!(
                "Too many arguments: {} (max {})",
                args.len(),
                security::MAX_ARGS_COUNT
            ));
        }

        let mut full_cmd = cmd.to_string();
        for arg in args {
            full_cmd.push(' ');
            // Sanitize each argument for safe cmd.exe /c usage
            let safe_arg = security::sanitize_windows_arg(arg)?;
            full_cmd.push_str(&safe_arg);
        }

        let mut c = Command::new("cmd.exe");
        c.arg("/c").arg(&full_cmd);
        c
    } else {
        // Linux/Mac: direct execution via execvp — no shell, safe by default
        security::validate_unix_args(args)?;
        let mut c = Command::new(cmd);
        c.args(args);
        c
    };

    // 4. Strip sensitive environment variables from child process
    // Instead of env_clear() (which breaks PATH, HOME, etc.), we selectively
    // set only safe env vars
    command.env_clear();
    for (key, value) in security::get_safe_env_vars() {
        command.env(&key, &value);
    }

    Ok(command)
}

/// Build a cross-platform Command for spawning (with piped I/O).
fn build_spawn_command(cmd: &str, args: &[String]) -> Result<Command, String> {
    let mut command = build_command(cmd, args)?;
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    Ok(command)
}

/// Parse a JSON array string into a Vec<String>.
/// Input: `["arg1", "arg2"]` or `"arg1"` (single string) or empty string.
///
/// Security: enforces MAX_ARGS_JSON_LEN input size limit.
pub fn parse_args_json(args_json: &str) -> Result<Vec<String>, String> {
    if args_json.is_empty() {
        return Ok(Vec::new());
    }

    // Input size limit
    if args_json.len() > security::MAX_ARGS_JSON_LEN {
        return Err(format!(
            "Arguments JSON too large: {} bytes (max {})",
            args_json.len(),
            security::MAX_ARGS_JSON_LEN
        ));
    }

    // Try parsing as JSON array of strings
    if let Ok(arr) = serde_json::from_str::<Vec<String>>(args_json) {
        if arr.len() > security::MAX_ARGS_COUNT {
            return Err(format!(
                "Too many arguments: {} (max {})",
                arr.len(),
                security::MAX_ARGS_COUNT
            ));
        }
        return Ok(arr);
    }

    // Try parsing as JSON array of mixed types
    if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(args_json) {
        if arr.len() > security::MAX_ARGS_COUNT {
            return Err(format!(
                "Too many arguments: {} (max {})",
                arr.len(),
                security::MAX_ARGS_COUNT
            ));
        }
        return Ok(arr
            .iter()
            .map(|v| match v {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .collect());
    }

    // Single string argument (not JSON array)
    if !args_json.starts_with('[') {
        return Ok(vec![args_json.to_string()]);
    }

    // Invalid JSON array
    Err(format!(
        "Invalid arguments JSON: {}",
        &args_json[..args_json.len().min(100)]
    ))
}

/// Run a command to completion and return JSON result.
/// `{ "exit_code": N, "stdout": "...", "stderr": "..." }`
///
/// Security: output is truncated at MAX_OUTPUT_SIZE.
pub async fn run_command(cmd: &str, args: &[String]) -> Result<String, String> {
    ffi_debug!("PROCESS", "Running: {} {:?}", cmd, args);

    let output = build_command(cmd, args)?
        .output()
        .await
        .map_err(|e| format!("Failed to execute '{}': {}", cmd, e))?;

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout_raw = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr_raw = String::from_utf8_lossy(&output.stderr).to_string();

    // Apply output size limits
    let stdout = security::truncate_output(&stdout_raw);
    let stderr = security::truncate_output(&stderr_raw);

    let stdout_escaped = serde_json::to_string(&stdout).unwrap_or_else(|_| "\"\"".to_owned());
    let stderr_escaped = serde_json::to_string(&stderr).unwrap_or_else(|_| "\"\"".to_owned());

    ffi_debug!("PROCESS", "Completed: {} (exit_code={})", cmd, exit_code);

    Ok(format!(
        r#"{{"exit_code":{},"stdout":{},"stderr":{}}}"#,
        exit_code, stdout_escaped, stderr_escaped
    ))
}

/// Run a command and return just stdout (trimmed).
pub async fn run_command_stdout(cmd: &str, args: &[String]) -> Result<String, String> {
    let output = build_command(cmd, args)?
        .output()
        .await
        .map_err(|e| format!("Failed to execute '{}': {}", cmd, e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(format!(
            "Command '{}' failed (exit {}): {}",
            cmd,
            output.status.code().unwrap_or(-1),
            stderr.trim()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(security::truncate_output(&stdout))
}

/// Spawn a long-running process. Returns the handle ID.
///
/// Security: enforces MAX_SPAWNED_PROCESSES limit to prevent resource exhaustion.
pub async fn spawn_process(cmd: &str, args: &[String]) -> Result<String, String> {
    ffi_debug!("PROCESS", "Spawning: {} {:?}", cmd, args);

    // Check concurrent process limit
    let current_count = get_registry().count();
    if current_count >= security::MAX_SPAWNED_PROCESSES {
        return Err(format!(
            "Too many spawned processes: {} (max {}). Kill existing processes first.",
            current_count,
            security::MAX_SPAWNED_PROCESSES
        ));
    }

    let child = build_spawn_command(cmd, args)?
        .spawn()
        .map_err(|e| format!("Failed to spawn '{}': {}", cmd, e))?;

    let handle_id = uuid::Uuid::new_v4().to_string();
    let handle = Arc::new(ProcessHandle::new(
        handle_id.clone(),
        cmd.to_string(),
        args.to_vec(),
        child,
    ));

    get_registry().insert(handle);

    ffi_debug!("PROCESS", "Spawned: {} -> handle {}", cmd, handle_id);
    Ok(handle_id)
}
