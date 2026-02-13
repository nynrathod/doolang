//! Process Helpers — Command execution and argument parsing.
//!
//! Single source of truth for running commands and parsing args.
//! Cross-platform: On Windows, shell built-in commands (echo, dir, etc.)
//! are transparently routed through cmd.exe /c.

use crate::handle::ProcessHandle;
use crate::registry::get_registry;

use doo_ffi_core::ffi_debug;
use std::sync::Arc;
use tokio::process::Command;

// ============================================================================
// Cross-platform command builder — Single Source of Truth
// ============================================================================
// On Windows, many common commands (echo, dir, type, etc.) are shell built-ins
// and cannot be executed directly. This helper transparently wraps them through
// cmd.exe /c, matching the behavior on Linux/Mac where these are real executables.
// ============================================================================

/// Build a cross-platform Command.
/// On Windows: routes through cmd.exe /c for shell built-in compatibility.
/// On Linux/Mac: executes directly (commands are real executables).
fn build_command(cmd: &str, args: &[String]) -> Command {
    if cfg!(windows) {
        // On Windows, use cmd.exe /c to handle shell built-ins transparently.
        // cmd.exe /c "<command> <args...>" runs any command — both built-ins
        // (echo, dir, type, set) and executables (git, node, etc.).
        let mut full_cmd = cmd.to_string();
        for arg in args {
            full_cmd.push(' ');
            // Quote args containing spaces
            if arg.contains(' ') || arg.contains('"') {
                full_cmd.push('"');
                full_cmd.push_str(&arg.replace('"', "\\\""));
                full_cmd.push('"');
            } else {
                full_cmd.push_str(arg);
            }
        }
        let mut command = Command::new("cmd.exe");
        command.arg("/c").arg(&full_cmd);
        command
    } else {
        let mut command = Command::new(cmd);
        command.args(args);
        command
    }
}

/// Build a cross-platform Command for spawning (with piped I/O).
fn build_spawn_command(cmd: &str, args: &[String]) -> Command {
    let mut command = build_command(cmd, args);
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    command
}

/// Parse a JSON array string into a Vec<String>.
/// Input: `["arg1", "arg2"]` or `"arg1"` (single string) or empty string.
pub fn parse_args_json(args_json: &str) -> Vec<String> {
    if args_json.is_empty() {
        return Vec::new();
    }

    // Try parsing as JSON array
    if let Ok(arr) = serde_json::from_str::<Vec<String>>(args_json) {
        return arr;
    }

    // Try parsing as JSON array of mixed types
    if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(args_json) {
        return arr
            .iter()
            .map(|v| match v {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .collect();
    }

    // Single string argument
    if !args_json.starts_with('[') {
        return vec![args_json.to_string()];
    }

    Vec::new()
}

/// Run a command to completion and return JSON result.
/// `{ "exit_code": N, "stdout": "...", "stderr": "..." }`
pub async fn run_command(cmd: &str, args: &[String]) -> Result<String, String> {
    ffi_debug!("PROCESS", "Running: {} {:?}", cmd, args);

    let output = build_command(cmd, args)
        .output()
        .await
        .map_err(|e| format!("Failed to execute '{}': {}", cmd, e))?;

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    let stdout_escaped = serde_json::to_string(&stdout).unwrap_or_default();
    let stderr_escaped = serde_json::to_string(&stderr).unwrap_or_default();

    ffi_debug!("PROCESS", "Completed: {} (exit_code={})", cmd, exit_code);

    Ok(format!(
        r#"{{"exit_code":{},"stdout":{},"stderr":{}}}"#,
        exit_code, stdout_escaped, stderr_escaped
    ))
}

/// Run a command and return just stdout (trimmed).
pub async fn run_command_stdout(cmd: &str, args: &[String]) -> Result<String, String> {
    let output = build_command(cmd, args)
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

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Spawn a long-running process. Returns the handle ID.
pub async fn spawn_process(cmd: &str, args: &[String]) -> Result<String, String> {
    ffi_debug!("PROCESS", "Spawning: {} {:?}", cmd, args);

    let child = build_spawn_command(cmd, args)
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
