//! LSP Server Entry Point
//!
//! Starts the Doo language server over stdio transport.

use anyhow::Result;
use lsp_server::{Connection, Message};
use lsp_types::InitializeParams;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    // Initialize logging (controlled by DOO_LSP_LOG env var)
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("DOO_LSP_LOG").unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    tracing::info!("Starting Doo Language Server");

    // Create LSP connection over stdio
    let (connection, io_threads) = Connection::stdio();

    // Initialize handshake
    let server_capabilities = doo_lsp::capabilities::server_capabilities();
    let init_params = match connection.initialize(serde_json::to_value(server_capabilities)?) {
        Ok(it) => it,
        Err(e) => {
            if e.channel_is_disconnected() {
                io_threads.join()?;
            }
            return Err(e.into());
        }
    };

    let init_params: InitializeParams = serde_json::from_value(init_params)?;

    tracing::info!("Doo LSP initialized");

    // Run the main event loop
    doo_lsp::handler::main_loop(&connection, init_params)?;

    // Clean shutdown
    io_threads.join()?;
    tracing::info!("Doo LSP shut down");

    Ok(())
}
