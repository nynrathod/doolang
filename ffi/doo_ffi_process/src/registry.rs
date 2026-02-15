//! Process Registry — Global concurrent registry of spawned processes.
//!
//! Uses DashMap for lock-free concurrent access.
//! Single source of truth for all active spawned processes.
//!
//! Lifecycle:
//! - insert() → adds process to registry
//! - kill_process() → kills process AND removes from registry
//! - wait_for_output() → waits for completion AND removes from registry
//! - Auto-cleanup: exited processes removed on status check

use crate::ensure_runtime;
use crate::handle::ProcessHandle;
use crate::security;

use dashmap::DashMap;
use doo_ffi_core::ffi_debug;
use std::sync::OnceLock;
use tokio::io::AsyncReadExt;

/// Global process registry.
pub struct ProcessRegistry {
    handles: DashMap<String, std::sync::Arc<ProcessHandle>>,
}

impl ProcessRegistry {
    pub fn new() -> Self {
        Self {
            handles: DashMap::new(),
        }
    }

    /// Insert a new process handle.
    pub fn insert(&self, handle: std::sync::Arc<ProcessHandle>) {
        let id = handle.id.clone();
        self.handles.insert(id.clone(), handle);
        ffi_debug!(
            "PROCESS",
            "Registered handle: {} (active: {})",
            id,
            self.handles.len()
        );
    }

    /// Kill a process by handle ID.
    /// Also removes the process from the registry to prevent leaks.
    pub fn kill_process(&self, handle_id: &str) -> Result<(), String> {
        let handle = self
            .handles
            .get(handle_id)
            .ok_or_else(|| format!("Process handle not found: {}", handle_id))?
            .clone();

        let rt = ensure_runtime();

        rt.block_on(async {
            let mut killed = handle.killed.lock().await;
            *killed = true;
            let mut child_guard = handle.child.lock().await;
            if let Some(ref mut child) = *child_guard {
                child
                    .kill()
                    .await
                    .map_err(|e| format!("Kill failed: {}", e))?;
            }
            Ok::<(), String>(())
        })?;

        // Remove from registry after kill — prevents leak
        self.handles.remove(handle_id);
        ffi_debug!(
            "PROCESS",
            "Killed and removed process: {} (active: {})",
            handle_id,
            self.handles.len()
        );
        Ok(())
    }

    /// Get process status as JSON string.
    /// Auto-removes exited processes from registry to prevent leaks.
    pub fn get_status(&self, handle_id: &str) -> Result<String, String> {
        let handle = self
            .handles
            .get(handle_id)
            .ok_or_else(|| format!("Process handle not found: {}", handle_id))?
            .clone();

        let rt = ensure_runtime();

        let result = rt.block_on(async {
            let exit = handle.exit_status.lock().await;
            if let Some(status) = &*exit {
                let code = status.code().unwrap_or(-1);
                return Ok((
                    format!(r#"{{"status":"exited","exit_code":{}}}"#, code),
                    true, // is_exited
                ));
            }
            drop(exit);

            // Try to check if child has exited
            let mut child_guard = handle.child.lock().await;
            if let Some(ref mut child) = *child_guard {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        let code = status.code().unwrap_or(-1);
                        let mut exit = handle.exit_status.lock().await;
                        *exit = Some(status);
                        Ok((
                            format!(r#"{{"status":"exited","exit_code":{}}}"#, code),
                            true,
                        ))
                    }
                    Ok(None) => Ok((
                        r#"{"status":"running","exit_code":null}"#.to_string(),
                        false,
                    )),
                    Err(e) => Err(format!("Status check failed: {}", e)),
                }
            } else {
                Ok((
                    r#"{"status":"unknown","exit_code":null}"#.to_string(),
                    true, // no child = already consumed, treat as exited
                ))
            }
        })?;

        let (json, is_exited) = result;

        // Auto-remove exited processes to prevent registry leak
        if is_exited {
            self.handles.remove(handle_id);
            ffi_debug!(
                "PROCESS",
                "Auto-removed exited process: {} (active: {})",
                handle_id,
                self.handles.len()
            );
        }

        Ok(json)
    }

    /// Wait for a process to finish and return output JSON.
    /// Removes the process from the registry after completion.
    pub async fn wait_for_output(&self, handle_id: &str) -> Result<String, String> {
        let handle = self
            .handles
            .get(handle_id)
            .ok_or_else(|| format!("Process handle not found: {}", handle_id))?
            .clone();

        let result = {
            let mut child_guard = handle.child.lock().await;
            if let Some(mut child) = child_guard.take() {
                // Read stdout (with size limit)
                let mut stdout_bytes = Vec::new();
                if let Some(ref mut stdout) = child.stdout {
                    // Read in chunks to enforce size limit
                    let mut buf = [0u8; 8192];
                    loop {
                        match stdout.read(&mut buf).await {
                            Ok(0) => break,
                            Ok(n) => {
                                if stdout_bytes.len() + n > security::MAX_OUTPUT_SIZE {
                                    let remaining = security::MAX_OUTPUT_SIZE - stdout_bytes.len();
                                    stdout_bytes.extend_from_slice(&buf[..remaining]);
                                    break;
                                }
                                stdout_bytes.extend_from_slice(&buf[..n]);
                            }
                            Err(_) => break,
                        }
                    }
                }

                // Read stderr (with size limit)
                let mut stderr_bytes = Vec::new();
                if let Some(ref mut stderr) = child.stderr {
                    let mut buf = [0u8; 8192];
                    loop {
                        match stderr.read(&mut buf).await {
                            Ok(0) => break,
                            Ok(n) => {
                                if stderr_bytes.len() + n > security::MAX_OUTPUT_SIZE {
                                    let remaining = security::MAX_OUTPUT_SIZE - stderr_bytes.len();
                                    stderr_bytes.extend_from_slice(&buf[..remaining]);
                                    break;
                                }
                                stderr_bytes.extend_from_slice(&buf[..n]);
                            }
                            Err(_) => break,
                        }
                    }
                }

                // Wait for exit
                let status = child
                    .wait()
                    .await
                    .map_err(|e| format!("Wait failed: {}", e))?;

                let code = status.code().unwrap_or(-1);
                let stdout_str = String::from_utf8_lossy(&stdout_bytes).to_string();
                let stderr_str = String::from_utf8_lossy(&stderr_bytes).to_string();

                // Store exit status
                let mut exit = handle.exit_status.lock().await;
                *exit = Some(status);

                // Store buffered output
                let mut stdout_buf = handle.stdout_buf.lock().await;
                stdout_buf.push_str(&stdout_str);
                let mut stderr_buf = handle.stderr_buf.lock().await;
                stderr_buf.push_str(&stderr_str);

                let stdout_escaped =
                    serde_json::to_string(&stdout_str).unwrap_or_else(|_| "\"\"".to_owned());
                let stderr_escaped =
                    serde_json::to_string(&stderr_str).unwrap_or_else(|_| "\"\"".to_owned());

                Ok(format!(
                    r#"{{"exit_code":{},"stdout":{},"stderr":{}}}"#,
                    code, stdout_escaped, stderr_escaped
                ))
            } else {
                // Child already consumed — return buffered data
                let stdout_buf = handle.stdout_buf.lock().await;
                let stderr_buf = handle.stderr_buf.lock().await;
                let exit = handle.exit_status.lock().await;
                let code = exit.as_ref().and_then(|s| s.code()).unwrap_or(-1);
                let stdout_escaped =
                    serde_json::to_string(&*stdout_buf).unwrap_or_else(|_| "\"\"".to_owned());
                let stderr_escaped =
                    serde_json::to_string(&*stderr_buf).unwrap_or_else(|_| "\"\"".to_owned());

                Ok(format!(
                    r#"{{"exit_code":{},"stdout":{},"stderr":{}}}"#,
                    code, stdout_escaped, stderr_escaped
                ))
            }
        };

        // Remove from registry after wait — process is done
        self.handles.remove(handle_id);
        ffi_debug!(
            "PROCESS",
            "Completed and removed process: {} (active: {})",
            handle_id,
            self.handles.len()
        );

        result
    }

    /// Check if a process is still running.
    /// Auto-removes exited processes from the registry.
    pub fn is_running(&self, handle_id: &str) -> bool {
        let handle = match self.handles.get(handle_id) {
            Some(h) => h.clone(),
            None => return false,
        };

        let rt = ensure_runtime();

        let (running, is_exited) = rt.block_on(async {
            // Check cached exit status first
            let exit = handle.exit_status.lock().await;
            if exit.is_some() {
                return (false, true);
            }
            drop(exit);

            // Try to poll child
            let mut child_guard = handle.child.lock().await;
            if let Some(ref mut child) = *child_guard {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        let mut exit = handle.exit_status.lock().await;
                        *exit = Some(status);
                        (false, true)
                    }
                    Ok(None) => (true, false),
                    Err(_) => (false, true),
                }
            } else {
                (false, true)
            }
        });

        // Auto-remove exited processes
        if is_exited {
            self.handles.remove(handle_id);
        }

        running
    }

    /// Read buffered stdout from a spawned process.
    pub fn read_stdout(&self, handle_id: &str) -> Result<String, String> {
        let handle = self
            .handles
            .get(handle_id)
            .ok_or_else(|| format!("Process handle not found: {}", handle_id))?
            .clone();

        let rt = ensure_runtime();

        rt.block_on(async {
            let buf = handle.stdout_buf.lock().await;
            Ok(buf.clone())
        })
    }

    /// Read buffered stderr from a spawned process.
    pub fn read_stderr(&self, handle_id: &str) -> Result<String, String> {
        let handle = self
            .handles
            .get(handle_id)
            .ok_or_else(|| format!("Process handle not found: {}", handle_id))?
            .clone();

        let rt = ensure_runtime();

        rt.block_on(async {
            let buf = handle.stderr_buf.lock().await;
            Ok(buf.clone())
        })
    }

    /// Shutdown all processes — kills all and clears registry.
    pub fn shutdown_all(&self) {
        let ids: Vec<String> = self.handles.iter().map(|e| e.key().clone()).collect();
        for id in &ids {
            let _ = self.kill_process(id);
        }
        // Clear any remaining (in case kill_process already removed some)
        self.handles.clear();
        ffi_debug!("PROCESS", "All processes shut down and registry cleared");
    }

    /// Number of tracked processes.
    pub fn count(&self) -> usize {
        self.handles.len()
    }

    /// Remove a handle from registry.
    pub fn remove(&self, handle_id: &str) {
        self.handles.remove(handle_id);
    }
}

/// Global process registry — single source of truth.
static PROCESS_REGISTRY: OnceLock<ProcessRegistry> = OnceLock::new();

pub fn get_registry() -> &'static ProcessRegistry {
    PROCESS_REGISTRY.get_or_init(ProcessRegistry::new)
}
