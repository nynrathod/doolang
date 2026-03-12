//! Process Handle — Represents a spawned background process.
//!
//! Stored in the global ProcessRegistry.
//! Tracks: child process, buffered stdout/stderr, exit status.

use std::process::ExitStatus;
use tokio::process::Child;
use tokio::sync::Mutex;

/// A spawned process handle with buffered I/O.
pub struct ProcessHandle {
    /// The unique handle ID (UUID v4)
    pub id: String,
    /// The command that was run
    pub command: String,
    /// The arguments passed
    pub args: Vec<String>,
    /// The child process (wrapped in Mutex for safe concurrent access)
    pub child: Mutex<Option<Child>>,
    /// Buffered stdout lines
    pub stdout_buf: Mutex<String>,
    /// Buffered stderr lines
    pub stderr_buf: Mutex<String>,
    /// Exit status once completed
    pub exit_status: Mutex<Option<ExitStatus>>,
    /// Whether kill was requested
    pub killed: Mutex<bool>,
}

impl ProcessHandle {
    pub fn new(id: String, command: String, args: Vec<String>, child: Child) -> Self {
        Self {
            id,
            command,
            args,
            child: Mutex::new(Some(child)),
            stdout_buf: Mutex::new(String::new()),
            stderr_buf: Mutex::new(String::new()),
            exit_status: Mutex::new(None),
            killed: Mutex::new(false),
        }
    }
}
