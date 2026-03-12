//! # WebSocket Module for Doo HTTP Server
//!
//! Production-grade WebSocket support integrated into the HTTP server.
//! Shares the same hyper server, Tokio runtime, and C-ABI patterns.
//!
//! ## Architecture
//!
//! - **Reuses hyper HTTP server** — WS upgrade happens during request handling
//! - **tokio-tungstenite** for WebSocket protocol
//! - **DashMap** registries for connections and rooms (lock-free)
//! - **Bounded mpsc channels** per connection (backpressure)
//! - **CancellationToken** per connection (auto-cleanup)
//! - **JSON event framing**: `{ "event": "...", "data": ... }`
//!
//! ## Submodules
//!
//! - `connection` — WsConnection type and handler signatures
//! - `config` — Centralized WS configuration
//! - `registry` — Connection & route registries
//! - `room` — Room membership management
//! - `handler` — Message dispatch and lifecycle events
//! - `upgrade` — HTTP → WebSocket upgrade and read/write loops

pub mod connection;
pub mod config;
pub mod registry;
pub mod room;
pub mod handler;
pub mod upgrade;

pub use connection::*;
pub use config::*;
pub use registry::*;
pub use room::*;
pub use handler::*;
pub use upgrade::*;
