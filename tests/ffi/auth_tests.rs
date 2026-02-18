//! Auth FFI Tests — Production-grade coverage
//! Full compiler pipeline: lex → parse → analyze → MIR → codegen
//! Syntax matches dev_test/http/8_jwt_middleware_test.doo: Database::Postgres(), Jwt()

use super::{assert_ffi_compiles, assert_ffi_compiles_with};

// =============================================================================
// 1. AUTH SETUP — signup + login
// =============================================================================

#[test]
fn auth_setup_basic() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
import std::Auth::{Jwt};
import std::Database;
struct User {
    id: Int @primary @auto,
    Email: Str @unique,
    Password: Str @hash @writeOnly,
}
fn main() {
    let db = Database::Postgres()?;
    let app = Server::new(":3000");
    app.auth("/signup", "/login", User, db);
    app.start();
}
"#,
        "signup",
    );
}

#[test]
fn auth_custom_paths() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
import std::Auth::{Jwt};
import std::Database;
struct User {
    id: Int @primary @auto,
    Email: Str @unique,
    Password: Str @hash @writeOnly,
    Name: Str,
}
fn main() {
    let db = Database::Postgres()?;
    let app = Server::new(":3000");
    app.auth("/api/register", "/api/login", User, db);
    app.start();
}
"#,
        "/api/register",
    );
}

// =============================================================================
// 2. JWT MIDDLEWARE PROTECTION
// =============================================================================

#[test]
fn auth_jwt_protect_single_route() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
import std::Auth::{Jwt};
import std::Database;
struct User {
    id: Int @primary @auto,
    Email: Str @unique,
    Password: Str @hash @writeOnly,
}
fn GetProfile() -> Str => "profile";
fn main() {
    let db = Database::Postgres()?;
    let app = Server::new(":3000");
    app.auth("/signup", "/login", User, db);
    app.get("/profile", Jwt(), GetProfile);
    app.start();
}
"#,
        "Jwt",
    );
}

#[test]
fn auth_jwt_protect_group() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
import std::Auth::{Jwt};
import std::Database;
struct User {
    id: Int @primary @auto,
    Email: Str @unique,
    Password: Str @hash @writeOnly,
}
fn ListItems() -> Str => "items";
fn CreateItem() -> Str => "created";
fn DeleteItem() -> Str => "deleted";
fn main() {
    let db = Database::Postgres()?;
    let app = Server::new(":3000");
    app.auth("/signup", "/login", User, db);
    app.group("/api", Jwt(), {
        get("/items", ListItems),
        post("/items", CreateItem),
        delete("/items/:id", DeleteItem),
    });
    app.start();
}
"#,
        "api",
    );
}

#[test]
fn auth_jwt_mixed_protected_and_public() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
import std::Auth::{Jwt};
import std::Database;
struct User {
    id: Int @primary @auto,
    Email: Str @unique,
    Password: Str @hash @writeOnly,
}
fn Ping() -> Str => "pong";
fn Health() -> Str => "ok";
fn GetProfile() -> Str => "profile";
fn GetSettings() -> Str => "settings";
fn main() {
    let db = Database::Postgres()?;
    let app = Server::new(":3000");
    app.auth("/signup", "/login", User, db);
    app.get("/ping", Ping);
    app.get("/health", Health);
    app.get("/profile", Jwt(), GetProfile);
    app.get("/settings", Jwt(), GetSettings);
    app.start();
}
"#,
        "settings",
    );
}

// =============================================================================
// 3. CUSTOM AUTH MIDDLEWARE
// =============================================================================

#[test]
fn auth_custom_middleware() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server, Request, Response, Next};
enum AuthError { Unauthorized, Forbidden }
fn AuthGuard(req: Request, next: Next) -> Response ! AuthError {
    let token = req.header("Authorization");
    if token == nil {
        return Err AuthError::Unauthorized;
    }
    let response = next.call();
    return Ok response;
}
fn ProtectedRoute() -> Str => "secret";
fn main() {
    let app = Server::new(":3000");
    app.get("/secret", AuthGuard, ProtectedRoute);
    app.start();
}
"#,
        "Authorization",
    );
}

#[test]
fn auth_custom_middleware_with_role_check() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server, Request, Response, Next};
enum AuthError { Unauthorized, Forbidden }
fn AdminOnly(req: Request, next: Next) -> Response ! AuthError {
    let role = req.header("X-Role");
    if role == nil {
        return Err AuthError::Unauthorized;
    }
    if role != "admin" {
        return Err AuthError::Forbidden;
    }
    let response = next.call();
    return Ok response;
}
fn AdminPanel() -> Str => "admin panel";
fn main() {
    let app = Server::new(":3000");
    app.get("/admin", AdminOnly, AdminPanel);
    app.start();
}
"#,
        "X-Role",
    );
}

#[test]
fn auth_stacked_middleware() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server, Request, Response, Next};
enum AuthError { Unauthorized, Forbidden }
fn AuthGuard(req: Request, next: Next) -> Response ! AuthError {
    if req.header("Authorization") == nil { return Err AuthError::Unauthorized; }
    let response = next.call();
    return Ok response;
}
fn AdminGuard(req: Request, next: Next) -> Response ! AuthError {
    if req.header("X-Role") != "admin" { return Err AuthError::Forbidden; }
    let response = next.call();
    return Ok response;
}
fn AdminAction() -> Str => "admin action done";
fn main() {
    let app = Server::new(":3000");
    app.get("/admin/action", AuthGuard, AdminGuard, AdminAction);
    app.start();
}
"#,
        "admin action done",
    );
}

// =============================================================================
// 4. AUTH WITH USER STRUCT VARIATIONS
// =============================================================================

#[test]
fn auth_user_with_extra_fields() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
import std::Auth::{Jwt};
import std::Database;
struct User {
    id: Int @primary @auto,
    Email: Str @email @unique,
    Password: Str @hash @writeOnly,
    Name: Str,
    Role: Str @default("user"),
    createdAt: Str @auto @readOnly,
    bio: Str?,
    avatar: Str?,
}
fn main() {
    let db = Database::Postgres()?;
    let app = Server::new(":3000");
    app.auth("/signup", "/login", User, db);
    app.crud("/users", User, db);
    app.start();
}
"#,
        "bio",
    );
}

// =============================================================================
// 5. FULL AUTH + CRUD PATTERN
// =============================================================================

#[test]
fn auth_full_app_pattern() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server, Request, Response, Next};
import std::Auth::{Jwt};
import std::Database;

struct User {
    id: Int @primary @auto,
    Email: Str @email @unique,
    Password: Str @hash @writeOnly,
    Name: Str,
    Role: Str @default("user"),
}

struct Post {
    id: Int @primary @auto,
    title: Str,
    content: Str,
    authorId: Int,
}

fn Ping() -> Str => "pong";
fn ListPosts() -> Str => "posts";
fn GetPost() -> Str => "post";
fn CreatePost() -> Str => "created";

fn main() {
    let db = Database::Postgres()?;
    let app = Server::new(":3000");
    app.cors();

    app.auth("/auth/signup", "/auth/login", User, db);

    app.get("/ping", Ping);

    app.group("/api", Jwt(), {
        get("/posts", ListPosts),
        get("/posts/:id", GetPost),
        post("/posts", CreatePost),
    });

    app.crud("/admin/users", User, db);
    app.start();
}
"#,
        "auth/login",
    );
}
