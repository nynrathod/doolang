//! WebSocket Upgrade — HTTP → WebSocket protocol upgrade.
//!
//! Integrates with the existing hyper HTTP server in `doo_ffi_http`.
//! When a request matches a registered WS route, the TCP connection
//! is upgraded to a WebSocket connection using tokio-tungstenite.
//!
//! Architecture:
//! - Reuses the hyper TcpListener from server.rs
//! - One async task per connection for reading frames
//! - One async task per connection for writing frames (from bounded channel)
//! - One async task for heartbeat ping/pong
//! - All tasks cancelled on disconnect via watch channel

use super::config::get_ws_config;
use super::connection::*;
use super::handler::*;
use super::registry::*;

use doo_ffi_core::ffi_debug;
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::protocol::Message;

/// Check if a path matches a registered WebSocket route.
pub fn is_ws_route(path: &str) -> bool {
    get_ws_registry().match_route(path).is_some()
}

/// Handle an already-upgraded WebSocket connection.
///
/// Called from server.rs after hyper performs the HTTP 101 upgrade.
/// The WebSocketStream is already established — no handshake needed here.
///
/// This function:
/// 1. Looks up the WS route handler
/// 2. Creates a new connection entry in ConnRegistry
/// 3. Calls the user's connection handler (registers event handlers)
/// 4. Fires onConnect
/// 5. Spawns read/write/heartbeat tasks
/// 6. On disconnect: fires onDisconnect, cleans up
pub async fn handle_ws_connection<S>(
    ws_stream: tokio_tungstenite::WebSocketStream<S>,
    path: &str,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{

    // Look up the route handler
    let handler = match get_ws_registry().match_route(path) {
        Some(h) => h,
        None => {
            ffi_debug!("WS", "No handler found for WS path: {}", path);
            return;
        }
    };

    // Generate unique connection ID
    let conn_id = uuid::Uuid::new_v4().to_string();
    ffi_debug!("WS", "New connection: {} on {}", conn_id, path);

    // Create send channel with backpressure
    let queue_size = get_ws_config().read().unwrap().send_queue_size;
    let (tx, rx) = mpsc::channel::<WsSendMessage>(queue_size);

    // Create cancellation watch channel for this connection's tasks
    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

    // Register connection in global registry
    get_conn_registry().insert(&conn_id, tx.clone(), cancel_tx);

    // Create the FFI-visible WsConnection handle
    // SAFETY: This pointer is used ONLY for the synchronous handler call below,
    // then freed before any .await points — avoids the !Send raw pointer issue.
    let ws_conn = Box::new(WsConnection {
        id: conn_id.clone(),
        closed: false,
    });
    let ws_conn_ptr = Box::into_raw(ws_conn);

    // Call user's connection handler — this registers event handlers
    // e.g., conn.on("message", ...), conn.onConnect(...), etc.
    // This is a synchronous FFI call — no await.
    handler(ws_conn_ptr as *const WsConnection);

    // Free the WsConnection handle immediately — it's no longer needed.
    // The conn_id string is used for all subsequent operations via registries.
    unsafe {
        let _ = Box::from_raw(ws_conn_ptr);
    }

    // Fire onConnect lifecycle event
    fire_on_connect(&conn_id);

    // Split the WebSocket stream into read and write halves
    let (write_half, read_half) = ws_stream.split();

    // Spawn the write task (processes outgoing messages from the channel)
    let write_cancel_rx = cancel_rx.clone();
    let write_conn_id = conn_id.clone();
    tokio::spawn(async move {
        write_loop(write_half, rx, write_cancel_rx, &write_conn_id).await;
    });

    // Spawn the heartbeat task
    let heartbeat_cancel_rx = cancel_rx.clone();
    let heartbeat_tx = tx.clone();
    let heartbeat_conn_id = conn_id.clone();
    tokio::spawn(async move {
        heartbeat_loop(heartbeat_tx, heartbeat_cancel_rx, &heartbeat_conn_id).await;
    });

    // Run the read loop (blocks until connection closes)
    read_loop(read_half, &conn_id, cancel_rx).await;

    // Connection closed — cleanup
    ffi_debug!("WS", "Connection closed: {}", conn_id);

    // Fire onDisconnect
    fire_on_disconnect(&conn_id);

    // Remove from registries (this also cancels tasks via watch channel and cleans rooms)
    get_conn_registry().remove(&conn_id);
}

/// Read loop — processes incoming WebSocket frames.
async fn read_loop<S>(
    mut read_half: S,
    conn_id: &str,
    mut cancel_rx: tokio::sync::watch::Receiver<bool>,
) where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let max_size = get_ws_config().read().unwrap().max_message_size;

    loop {
        tokio::select! {
            result = cancel_rx.changed() => {
                if result.is_ok() && *cancel_rx.borrow() {
                    ffi_debug!("WS", "Read loop cancelled for {}", conn_id);
                    break;
                }
            }
            frame = read_half.next() => {
                match frame {
                    Some(Ok(msg)) => {
                        match msg {
                            Message::Text(text) => {
                                if text.len() > max_size {
                                    ffi_debug!("WS", "Message too large from {} ({} > {})", conn_id, text.len(), max_size);
                                    fire_on_error(conn_id, &format!("Message exceeds max size: {} > {}", text.len(), max_size));
                                    get_conn_registry().close(conn_id);
                                    break;
                                }
                                handle_text_message(conn_id, &text);
                            }
                            Message::Binary(data) => {
                                if data.len() > max_size {
                                    ffi_debug!("WS", "Binary too large from {} ({} > {})", conn_id, data.len(), max_size);
                                    fire_on_error(conn_id, &format!("Binary message exceeds max size: {} > {}", data.len(), max_size));
                                    get_conn_registry().close(conn_id);
                                    break;
                                }
                                handle_binary_message(conn_id, &data);
                            }
                            Message::Ping(_) => {
                                // Tungstenite auto-responds to pings with pong
                                ffi_debug!("WS", "Ping received from {}", conn_id);
                            }
                            Message::Pong(_) => {
                                // Pong received — connection is alive
                                ffi_debug!("WS", "Pong received from {}", conn_id);
                            }
                            Message::Close(_) => {
                                ffi_debug!("WS", "Close frame from {}", conn_id);
                                break;
                            }
                            _ => {
                                // Frame type not handled (e.g., Frame)
                            }
                        }
                    }
                    Some(Err(e)) => {
                        ffi_debug!("WS", "Read error from {}: {}", conn_id, e);
                        fire_on_error(conn_id, &format!("Read error: {}", e));
                        break;
                    }
                    None => {
                        // Stream ended
                        ffi_debug!("WS", "Stream ended for {}", conn_id);
                        break;
                    }
                }
            }
        }
    }
}

/// Write loop — sends outgoing messages from the bounded channel to the WebSocket.
async fn write_loop<S>(
    mut write_half: S,
    mut rx: mpsc::Receiver<WsSendMessage>,
    mut cancel_rx: tokio::sync::watch::Receiver<bool>,
    conn_id: &str,
) where
    S: SinkExt<Message> + Unpin,
    <S as futures_util::Sink<Message>>::Error: std::fmt::Display,
{
    loop {
        tokio::select! {
            result = cancel_rx.changed() => {
                if result.is_ok() && *cancel_rx.borrow() {
                    // Send close frame before exiting
                    let _ = write_half.send(Message::Close(None)).await;
                    ffi_debug!("WS", "Write loop cancelled for {}", conn_id);
                    break;
                }
            }
            msg = rx.recv() => {
                match msg {
                    Some(WsSendMessage::Text(text)) => {
                        if let Err(e) = write_half.send(Message::Text(text.into())).await {
                            ffi_debug!("WS", "Write error for {}: {}", conn_id, e);
                            break;
                        }
                    }
                    Some(WsSendMessage::Binary(data)) => {
                        if let Err(e) = write_half.send(Message::Binary(data.into())).await {
                            ffi_debug!("WS", "Write error for {}: {}", conn_id, e);
                            break;
                        }
                    }
                    Some(WsSendMessage::Ping) => {
                        if let Err(e) = write_half.send(Message::Ping(Vec::new().into())).await {
                            ffi_debug!("WS", "Ping write error for {}: {}", conn_id, e);
                            break;
                        }
                    }
                    Some(WsSendMessage::Close) => {
                        let _ = write_half.send(Message::Close(None)).await;
                        ffi_debug!("WS", "Close sent for {}", conn_id);
                        break;
                    }
                    None => {
                        // Channel closed
                        break;
                    }
                }
            }
        }
    }
}

/// Heartbeat loop — sends periodic pings to detect dead connections.
async fn heartbeat_loop(
    tx: mpsc::Sender<WsSendMessage>,
    mut cancel_rx: tokio::sync::watch::Receiver<bool>,
    conn_id: &str,
) {
    let interval_secs = get_ws_config().read().unwrap().heartbeat_interval_secs;
    let interval = Duration::from_secs(interval_secs);

    loop {
        tokio::select! {
            result = cancel_rx.changed() => {
                if result.is_ok() && *cancel_rx.borrow() {
                    ffi_debug!("WS", "Heartbeat cancelled for {}", conn_id);
                    break;
                }
            }
            _ = tokio::time::sleep(interval) => {
                // Send a Ping frame via the write channel
                if tx.is_closed() {
                    ffi_debug!("WS", "Heartbeat: channel closed for {}", conn_id);
                    break;
                }
                if tx.try_send(WsSendMessage::Ping).is_err() {
                    ffi_debug!("WS", "Heartbeat: ping send failed for {} (backpressure or closed)", conn_id);
                    break;
                }
                ffi_debug!("WS", "Heartbeat ping sent for {}", conn_id);
            }
        }
    }
}
