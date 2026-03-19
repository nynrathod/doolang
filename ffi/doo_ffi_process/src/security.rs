//! Process Security — Single Source of Truth for command validation.
//!
//! Production-grade security for DooCloud:
//! - Windows shell injection prevention (cmd.exe /c)
//! - Command validation & path traversal prevention
//! - Docker argument sanitization
//! - Environment variable leak prevention
//! - Input size limits
//! - Output size limits

use doo_ffi_core::ffi_debug;

// ============================================================================
// Constants — Centralized limits (Single Source of Truth)
// ============================================================================

/// Maximum command string length (prevents DoS via huge command strings)
pub const MAX_COMMAND_LEN: usize = 4096;

/// Maximum single argument length
pub const MAX_ARG_LEN: usize = 32_768; // 32KB per arg (for JSON payloads etc.)

/// Maximum number of arguments
pub const MAX_ARGS_COUNT: usize = 1024;

/// Maximum total args JSON input size
pub const MAX_ARGS_JSON_LEN: usize = 1_048_576; // 1MB

/// Maximum output size per stream (stdout/stderr) — prevents OOM on huge output
pub const MAX_OUTPUT_SIZE: usize = 50 * 1024 * 1024; // 50MB

/// Maximum concurrent spawned processes
pub const MAX_SPAWNED_PROCESSES: usize = 256;

// ============================================================================
// Windows Shell Injection Prevention
// ============================================================================

/// Characters that are dangerous when passed through `cmd.exe /c`.
/// These allow command chaining, piping, redirection, and variable expansion.
const WINDOWS_DANGEROUS_CHARS: &[char] = &[
    '&', // Command separator: echo hi & del /s C:\
    '|', // Pipe: echo hi | malicious
    '>', // Output redirect: echo hi > C:\Windows\system32\file
    '<', // Input redirect
    '^', // Escape character in cmd.exe
    '`', // Backtick (PowerShell)
    ';', // Command separator (some shells)
    '!', // Delayed expansion
    '%', // Environment variable expansion: %PATH%
    '(', // Subshell grouping
    ')', // Subshell grouping
];

/// Characters dangerous in Unix shell contexts (if we ever use sh -c).
const UNIX_DANGEROUS_CHARS: &[char] = &[
    ';', '&', '|', '>', '<', '`', '$', '(', ')', '{', '}', '!', '\\', '\n', '\r',
];

/// Sanitize a single argument for safe use with `cmd.exe /c` on Windows.
/// Only wraps in double quotes when necessary (contains spaces or quotes).
/// Rejects arguments containing shell metacharacters that cannot be safely escaped.
pub fn sanitize_windows_arg(arg: &str) -> Result<String, String> {
    // Check length
    if arg.len() > MAX_ARG_LEN {
        return Err(format!(
            "Argument too long: {} bytes (max {})",
            arg.len(),
            MAX_ARG_LEN
        ));
    }

    // Reject dangerous characters that can't be safely quoted in cmd.exe
    for ch in WINDOWS_DANGEROUS_CHARS {
        if arg.contains(*ch) {
            return Err(format!(
                "Argument contains unsafe character '{}' for Windows shell execution. \
                 Use direct execution on Linux/Mac or avoid shell metacharacters.",
                ch
            ));
        }
    }

    // Only quote args that need it (contain spaces or double quotes)
    if arg.contains(' ') || arg.contains('"') {
        let escaped = arg.replace('"', "\\\"");
        Ok(format!("\"{}\"", escaped))
    } else {
        Ok(arg.to_string())
    }
}

/// Validate arguments for direct execution (Linux/Mac).
/// Less restrictive since Command::new + .args() doesn't use a shell.
pub fn validate_unix_args(args: &[String]) -> Result<(), String> {
    if args.len() > MAX_ARGS_COUNT {
        return Err(format!(
            "Too many arguments: {} (max {})",
            args.len(),
            MAX_ARGS_COUNT
        ));
    }
    for (i, arg) in args.iter().enumerate() {
        if arg.len() > MAX_ARG_LEN {
            return Err(format!(
                "Argument {} too long: {} bytes (max {})",
                i,
                arg.len(),
                MAX_ARG_LEN
            ));
        }
    }
    Ok(())
}

// ============================================================================
// Command Validation
// ============================================================================

/// Validate a command string.
/// Prevents:
/// - Empty commands
/// - Path traversal (../../bin/evil)
/// - Commands with slashes pointing to arbitrary executables (partial — full
///   allowlisting is for DooCloud mode)
pub fn validate_command(cmd: &str) -> Result<(), String> {
    if cmd.is_empty() {
        return Err("Command cannot be empty".to_string());
    }

    if cmd.len() > MAX_COMMAND_LEN {
        return Err(format!(
            "Command too long: {} bytes (max {})",
            cmd.len(),
            MAX_COMMAND_LEN
        ));
    }

    // Prevent null bytes (could truncate strings in C layer)
    if cmd.contains('\0') {
        return Err("Command contains null byte".to_string());
    }

    // Prevent path traversal
    if cmd.contains("..") {
        return Err("Command contains path traversal (..)".to_string());
    }

    // Prevent newlines (could inject commands in some shells)
    if cmd.contains('\n') || cmd.contains('\r') {
        return Err("Command contains newline characters".to_string());
    }

    Ok(())
}

// ============================================================================
// Docker Argument Sanitization (DooCloud)
// ============================================================================

/// Docker flags that are forbidden in a cloud/multi-tenant environment.
/// These could allow container escape, host access, or privilege escalation.
const FORBIDDEN_DOCKER_FLAGS: &[&str] = &[
    "--privileged",
    "--cap-add",
    "--security-opt",
    "--pid=host",
    "--network=host",
    "--userns=host",
    "--uts=host",
    "--ipc=host",
    "--device",
    "--cap-drop=ALL", // Ironically, this followed by --cap-add is a bypass pattern
];

/// Docker volume mount patterns that could expose the host filesystem.
const FORBIDDEN_VOLUME_PREFIXES: &[&str] = &[
    "/:/",                    // Mount entire root
    "/etc:/",                 // Mount /etc
    "/var/run/docker.sock",   // Docker socket access = full host control
    "/proc:/",                // Mount /proc
    "/sys:/",                 // Mount /sys
    "//./pipe/docker_engine", // Windows Docker socket
];

/// Validate Docker arguments for cloud safety.
/// Only called when command is "docker" and cloud mode is enabled.
pub fn validate_docker_args(args: &[String]) -> Result<(), String> {
    for (i, arg) in args.iter().enumerate() {
        // Check forbidden flags
        let arg_lower = arg.to_lowercase();
        for forbidden in FORBIDDEN_DOCKER_FLAGS {
            if arg_lower.starts_with(&forbidden.to_lowercase()) {
                return Err(format!(
                    "Docker flag '{}' is not allowed in cloud environment",
                    arg
                ));
            }
        }

        // Check volume mounts (-v and --mount)
        if (arg == "-v" || arg == "--volume") && i + 1 < args.len() {
            let mount = &args[i + 1];
            for prefix in FORBIDDEN_VOLUME_PREFIXES {
                if mount.starts_with(prefix) {
                    return Err(format!(
                        "Docker volume mount '{}' is not allowed in cloud environment",
                        mount
                    ));
                }
            }
        }

        // Inline -v flag (e.g., -v=/host:/container)
        if arg.starts_with("-v=") || arg.starts_with("--volume=") {
            let mount_part = arg.split('=').nth(1).unwrap_or("");
            for prefix in FORBIDDEN_VOLUME_PREFIXES {
                if mount_part.starts_with(prefix) {
                    return Err(format!(
                        "Docker volume mount '{}' is not allowed in cloud environment",
                        mount_part
                    ));
                }
            }
        }
    }

    ffi_debug!("PROCESS", "Docker args validated: {:?}", args);
    Ok(())
}

// ============================================================================
// Environment Variable Safety
// ============================================================================

/// Environment variables that should NEVER be passed to child processes.
/// These contain secrets that could be exfiltrated.
const SENSITIVE_ENV_VARS: &[&str] = &[
    "JWT_SECRET",
    "DATABASE_URL",
    "DB_PASSWORD",
    "DB_URL",
    "SECRET_KEY",
    "API_KEY",
    "API_SECRET",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "GITHUB_TOKEN",
    "STRIPE_SECRET_KEY",
    "SENDGRID_API_KEY",
    "PRIVATE_KEY",
    "ENCRYPTION_KEY",
    "AUTH_SECRET",
    "SESSION_SECRET",
    "COOKIE_SECRET",
    "SIGNING_KEY",
    "MASTER_KEY",
];

/// Get safe environment variables for child processes.
/// Strips sensitive variables to prevent secret leakage.
/// Returns: Vec<(key, value)> of safe env vars.
pub fn get_safe_env_vars() -> Vec<(String, String)> {
    std::env::vars()
        .filter(|(key, _)| !is_sensitive_key(key))
        .collect()
}

/// Check if an environment variable key is sensitive (should not be passed to child processes).
fn is_sensitive_key(key: &str) -> bool {
    let key_upper = key.to_uppercase();
    SENSITIVE_ENV_VARS
        .iter()
        .any(|s| key_upper.contains(s))
        || key_upper.ends_with("_SECRET")
        || key_upper.ends_with("_PASSWORD")
        || key_upper.ends_with("_TOKEN")
        || key_upper.ends_with("_PRIVATE_KEY")
}

/// Remove sensitive environment variables from a Command.
/// Preferred over env_clear() + selective re-add because it preserves all
/// platform-specific system variables (PATH, PATHEXT, SystemRoot, COMSPEC, etc.)
/// that are required for proper command execution on Windows and Linux.
pub fn remove_sensitive_env_vars(command: &mut tokio::process::Command) {
    for (key, _) in std::env::vars() {
        if is_sensitive_key(&key) {
            command.env_remove(&key);
        }
    }
}

// ============================================================================
// Output Truncation
// ============================================================================

/// Truncate output to MAX_OUTPUT_SIZE and append a warning if truncated.
pub fn truncate_output(output: &str) -> String {
    if output.len() <= MAX_OUTPUT_SIZE {
        output.to_string()
    } else {
        let truncated = &output[..MAX_OUTPUT_SIZE];
        format!(
            "{}...\n[OUTPUT TRUNCATED: {} bytes total, showing first {} bytes]",
            truncated,
            output.len(),
            MAX_OUTPUT_SIZE
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_command_empty() {
        assert!(validate_command("").is_err());
    }

    #[test]
    fn test_validate_command_null_byte() {
        assert!(validate_command("echo\0evil").is_err());
    }

    #[test]
    fn test_validate_command_path_traversal() {
        assert!(validate_command("../../bin/sh").is_err());
    }

    #[test]
    fn test_validate_command_newline() {
        assert!(validate_command("echo\nrm -rf /").is_err());
    }

    #[test]
    fn test_validate_command_valid() {
        assert!(validate_command("echo").is_ok());
        assert!(validate_command("docker").is_ok());
        assert!(validate_command("git").is_ok());
    }

    #[test]
    fn test_sanitize_windows_arg_safe() {
        // No spaces — should NOT be quoted
        let result = sanitize_windows_arg("hello");
        assert_eq!(result.unwrap(), "hello");
        // With spaces — should be quoted
        let result = sanitize_windows_arg("hello world");
        assert_eq!(result.unwrap(), "\"hello world\"");
    }

    #[test]
    fn test_sanitize_windows_arg_injection() {
        // All of these should be rejected
        assert!(sanitize_windows_arg("hello & del /s C:\\").is_err());
        assert!(sanitize_windows_arg("hello | evil").is_err());
        assert!(sanitize_windows_arg("hello > file").is_err());
        assert!(sanitize_windows_arg("%PATH%").is_err());
        assert!(sanitize_windows_arg("hello;evil").is_err());
    }

    #[test]
    fn test_docker_forbidden_privileged() {
        let args = vec![
            "run".to_string(),
            "--privileged".to_string(),
            "image".to_string(),
        ];
        assert!(validate_docker_args(&args).is_err());
    }

    #[test]
    fn test_docker_forbidden_socket_mount() {
        let args = vec![
            "run".to_string(),
            "-v".to_string(),
            "/var/run/docker.sock:/var/run/docker.sock".to_string(),
            "image".to_string(),
        ];
        assert!(validate_docker_args(&args).is_err());
    }

    #[test]
    fn test_docker_safe_args() {
        let args = vec![
            "run".to_string(),
            "-p".to_string(),
            "8080:80".to_string(),
            "nginx".to_string(),
        ];
        assert!(validate_docker_args(&args).is_ok());
    }

    #[test]
    fn test_safe_env_vars_filters_secrets() {
        // This test checks the filtering logic works
        // We can't easily set env vars in a test, but we can verify the filter
        let vars = get_safe_env_vars();
        for (key, _) in &vars {
            let upper = key.to_uppercase();
            assert!(!upper.contains("JWT_SECRET"), "JWT_SECRET leaked");
            assert!(!upper.contains("DATABASE_URL"), "DATABASE_URL leaked");
        }
    }
}
