//! Process Registry — Global concurrent registry of spawned processes.
//!
//! Uses DashMap for lock-free concurrent access.
//! Single source of truth for all active spawned processes.

use crate::ensure_runtime;
use crate::handle::ProcessHandle;

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
    pub fn kill_process(&self, handle_id: &str) -> Result<(), String> {
        let handle = self
            .handles
            .get(handle_id)
            .ok_or_else(|| format!("Process handle not found: {}", handle_id))?
            .clone();

        // Use try_lock to avoid blocking in sync context
        // We need to use the runtime to kill
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

        ffi_debug!("PROCESS", "Killed process: {}", handle_id);
        Ok(())
    }

    /// Get process status as JSON string.
    pub fn get_status(&self, handle_id: &str) -> Result<String, String> {
        let handle = self
            .handles
            .get(handle_id)
            .ok_or_else(|| format!("Process handle not found: {}", handle_id))?
            .clone();

        let rt = ensure_runtime();

        rt.block_on(async {
            let exit = handle.exit_status.lock().await;
            if let Some(status) = &*exit {
                let code = status.code().unwrap_or(-1);
                return Ok(format!(r#"{{"status":"exited","exit_code":{}}}"#, code));
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
                        Ok(format!(r#"{{"status":"exited","exit_code":{}}}"#, code))
                    }
                    Ok(None) => Ok(r#"{"status":"running","exit_code":null}"#.to_string()),
                    Err(e) => Err(format!("Status check failed: {}", e)),
                }
            } else {
                Ok(r#"{"status":"unknown","exit_code":null}"#.to_string())
            }
        })
    }

    /// Wait for a process to finish and return output JSON.
    pub async fn wait_for_output(&self, handle_id: &str) -> Result<String, String> {
        let handle = self
            .handles
            .get(handle_id)
            .ok_or_else(|| format!("Process handle not found: {}", handle_id))?
            .clone();

        let mut child_guard = handle.child.lock().await;
        if let Some(mut child) = child_guard.take() {
            // Read stdout
            let mut stdout_str = String::new();
            if let Some(ref mut stdout) = child.stdout {
                let _ = stdout.read_to_string(&mut stdout_str).await;
            }

            // Read stderr
            let mut stderr_str = String::new();
            if let Some(ref mut stderr) = child.stderr {
                let _ = stderr.read_to_string(&mut stderr_str).await;
            }

            // Wait for exit
            let status = child
                .wait()
                .await
                .map_err(|e| format!("Wait failed: {}", e))?;

            let code = status.code().unwrap_or(-1);

            // Store exit status
            let mut exit = handle.exit_status.lock().await;
            *exit = Some(status);

            // Store buffered output
            let mut stdout_buf = handle.stdout_buf.lock().await;
            stdout_buf.push_str(&stdout_str);
            let mut stderr_buf = handle.stderr_buf.lock().await;
            stderr_buf.push_str(&stderr_str);

            let stdout_escaped = serde_json::to_string(&stdout_str).unwrap_or_default();
            let stderr_escaped = serde_json::to_string(&stderr_str).unwrap_or_default();

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
            let stdout_escaped = serde_json::to_string(&*stdout_buf).unwrap_or_default();
            let stderr_escaped = serde_json::to_string(&*stderr_buf).unwrap_or_default();

            Ok(format!(
                r#"{{"exit_code":{},"stdout":{},"stderr":{}}}"#,
                code, stdout_escaped, stderr_escaped
            ))
        }
    }

    /// Check if a process is still running.
    pub fn is_running(&self, handle_id: &str) -> bool {
        let handle = match self.handles.get(handle_id) {
            Some(h) => h.clone(),
            None => return false,
        };

        let rt = ensure_runtime();

        rt.block_on(async {
            // Check cached exit status first
            let exit = handle.exit_status.lock().await;
            if exit.is_some() {
                return false;
            }
            drop(exit);

            // Try to poll child
            let mut child_guard = handle.child.lock().await;
            if let Some(ref mut child) = *child_guard {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        let mut exit = handle.exit_status.lock().await;
                        *exit = Some(status);
                        false
                    }
                    Ok(None) => true,
                    Err(_) => false,
                }
            } else {
                false
            }
        })
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

    /// Shutdown all processes.
    pub fn shutdown_all(&self) {
        let ids: Vec<String> = self.handles.iter().map(|e| e.key().clone()).collect();
        for id in &ids {
            let _ = self.kill_process(id);
        }
        self.handles.clear();
        ffi_debug!("PROCESS", "All processes shut down");
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
