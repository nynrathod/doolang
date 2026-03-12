//! WebSocket FFI Tests — Production-grade coverage
//! Full compiler pipeline: lex → parse → analyze → MIR → codegen
//! Syntax matches dev_test/websocket/main.doo

use super::{assert_ffi_compiles, assert_ffi_compiles_with};

// =============================================================================
// 1. WEBSOCKET IMPORTS
// =============================================================================

#[test]
fn ws_import_connection() {
    assert_ffi_compiles("import std::Http::{Server, WsConnection}; fn main() { }");
}

#[test]
fn ws_import_with_error() {
    assert_ffi_compiles("import std::Http::{Server, WsConnection, WsError}; fn main() { }");
}

// =============================================================================
// 2. WEBSOCKET ROUTE REGISTRATION
// =============================================================================

#[test]
fn ws_echo_handler() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server, WsConnection};
fn onEchoMessage(conn: WsConnection, data: Str) {
    conn.emit("echo", data);
}
fn echoHandler(conn: WsConnection) {
    conn.on("echo", onEchoMessage);
}
fn main() {
    let app = Server::new(":3000");
    app.ws("/ws/echo", echoHandler);
    app.start();
}
"#,
        "echo",
    );
}

// =============================================================================
// 3. WEBSOCKET EVENT HANDLERS
// =============================================================================

#[test]
fn ws_on_message() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server, WsConnection};
fn onMsg(conn: WsConnection, data: Str) {
    conn.emit("reply", data);
}
fn handler(conn: WsConnection) {
    conn.on("message", onMsg);
}
fn main() {
    let app = Server::new(":3000");
    app.ws("/ws/chat", handler);
    app.start();
}
"#,
        "message",
    );
}

#[test]
fn ws_multiple_events() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server, WsConnection};
fn onChat(conn: WsConnection, data: Str) {
    conn.emit("chat_reply", data);
}
fn onJoin(conn: WsConnection, data: Str) {
    conn.join(data);
}
fn onLeave(conn: WsConnection, data: Str) {
    conn.leave(data);
}
fn chatHandler(conn: WsConnection) {
    conn.on("message", onChat);
    conn.on("join_room", onJoin);
    conn.on("leave_room", onLeave);
}
fn main() {
    let app = Server::new(":3000");
    app.ws("/ws/chat", chatHandler);
    app.start();
}
"#,
        "join_room",
    );
}

// =============================================================================
// 4. WEBSOCKET ROOMS
// =============================================================================

#[test]
fn ws_room_join_and_emit() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server, WsConnection};
fn onRoomMsg(conn: WsConnection, data: Str, app: Server) {
    app.toRoomEmit("lobby", "message", data);
}
fn handler(conn: WsConnection) {
    conn.join("lobby");
    conn.on("message", onRoomMsg);
}
fn main() {
    let app = Server::new(":3000");
    app.ws("/ws/rooms", handler);
    app.start();
}
"#,
        "lobby",
    );
}

// =============================================================================
// 5. WEBSOCKET LIFECYCLE HOOKS
// =============================================================================

#[test]
fn ws_lifecycle_hooks() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server, WsConnection};
fn onConnect(conn: WsConnection) {
    print("connected");
}
fn onDisconnect(conn: WsConnection) {
    print("disconnected");
}
fn onError(conn: WsConnection, err: Str) {
    print("error: " + err);
}
fn onPing(conn: WsConnection, data: Str) {
    conn.emit("pong", data);
}
fn lifecycleHandler(conn: WsConnection) {
    conn.onConnect(onConnect);
    conn.onDisconnect(onDisconnect);
    conn.onError(onError);
    conn.on("ping", onPing);
}
fn main() {
    let app = Server::new(":3000");
    app.ws("/ws/lifecycle", lifecycleHandler);
    app.start();
}
"#,
        "connected",
    );
}

// =============================================================================
// 6. WEBSOCKET CONNECTION STATE
// =============================================================================

#[test]
fn ws_close_and_is_closed() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server, WsConnection};
fn onCheck(conn: WsConnection, data: Str) {
    let closed = conn.isClosed();
    if closed {
        conn.emit("status", "already_closed");
    } else {
        conn.emit("status", "open");
    }
}
fn onClose(conn: WsConnection, data: Str) {
    conn.emit("closing", "bye");
    conn.close();
}
fn closeHandler(conn: WsConnection) {
    conn.on("check_status", onCheck);
    conn.on("server_close", onClose);
}
fn main() {
    let app = Server::new(":3000");
    app.ws("/ws/close-test", closeHandler);
    app.start();
}
"#,
        "already_closed",
    );
}

// =============================================================================
// 7. WEBSOCKET SERVER-LEVEL FEATURES
// =============================================================================

#[test]
fn ws_active_connections() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server, WsConnection};
fn onEcho(conn: WsConnection, data: Str) {
    conn.emit("echo", data);
}
fn handler(conn: WsConnection) {
    conn.on("echo", onEcho);
}
fn getConnections(app: Server) -> Str {
    let count = app.activeWsConnections();
    return "${count}";
}
fn main() {
    let app = Server::new(":3000");
    app.ws("/ws/echo", handler);
    app.get("/status", getConnections);
    app.start();
}
"#,
        "getConnections",
    );
}

#[test]
fn ws_broadcast() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server, WsConnection};
fn handler(conn: WsConnection) {
    conn.on("msg", (conn: WsConnection, data: Str) => { conn.emit("msg", data); });
}
fn doBroadcast(app: Server) -> Str {
    app.broadcast("server_event", "hello_all");
    return "broadcast_sent";
}
fn main() {
    let app = Server::new(":3000");
    app.ws("/ws/chat", handler);
    app.get("/broadcast", doBroadcast);
    app.start();
}
"#,
        "broadcast",
    );
}

// =============================================================================
// 8. WEBSOCKET + HTTP MIXED
// =============================================================================

#[test]
fn ws_full_app_pattern() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server, WsConnection};

fn onEchoMessage(conn: WsConnection, data: Str) {
    conn.emit("echo", data);
}

fn echoHandler(conn: WsConnection) {
    conn.on("echo", onEchoMessage);
}

fn onChatMessage(conn: WsConnection, data: Str, app: Server) {
    app.toRoomEmit("lobby", "message", data);
}

fn chatHandler(conn: WsConnection) {
    conn.join("lobby");
    conn.on("message", onChatMessage);
}

fn getStatus(app: Server) -> Str {
    let count = app.activeWsConnections();
    return "active: ${count}";
}

fn main() {
    let app = Server::new(":3210");
    app.ws("/ws/echo", echoHandler);
    app.ws("/ws/chat", chatHandler);
    app.get("/status", getStatus);
    app.start();
}
"#,
        "3210",
    );
}
