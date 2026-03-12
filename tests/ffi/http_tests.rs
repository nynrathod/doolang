//! HTTP FFI Tests — Production-grade coverage
//! Full compiler pipeline: lex → parse → analyze → MIR → codegen
//! Syntax matches dev_test/http/: Server::new, Database::Postgres(), Jwt()

use super::{assert_ffi_compiles, assert_ffi_compiles_with};

// =============================================================================
// 1. IMPORTS
// =============================================================================

#[test]
fn http_import_server() {
    assert_ffi_compiles("import std::Http::{Server}; fn main() { }");
}

#[test]
fn http_import_multiple() {
    assert_ffi_compiles("import std::Http::{Server, Request, Response}; fn main() { }");
}

#[test]
fn http_import_star() {
    assert_ffi_compiles("import std::Http::*; fn main() { }");
}

#[test]
fn http_import_alias() {
    assert_ffi_compiles("import std::Http::{Server as HttpServer}; fn main() { }");
}

// =============================================================================
// 2. SERVER CREATION
// =============================================================================

#[test]
fn http_server_new() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
fn main() {
    let app = Server::new(":3000");
    app.start();
}
"#,
        ":3000",
    );
}

#[test]
fn http_server_new_with_host() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
fn main() {
    let app = Server::new("127.0.0.1:8080");
    app.start();
}
"#,
        "127.0.0.1:8080",
    );
}

// =============================================================================
// 3. ROUTE REGISTRATION
// =============================================================================

#[test]
fn http_get_route() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
fn Ping() -> Str => "pong";
fn main() {
    let app = Server::new(":3000");
    app.get("/ping", Ping);
    app.start();
}
"#,
        "/ping",
    );
}

#[test]
fn http_post_route() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
fn CreateUser() -> Str => "created";
fn main() {
    let app = Server::new(":3000");
    app.post("/users", CreateUser);
    app.start();
}
"#,
        "/users",
    );
}

#[test]
fn http_put_route() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
fn UpdateUser() -> Str => "updated";
fn main() {
    let app = Server::new(":3000");
    app.put("/users/:id", UpdateUser);
    app.start();
}
"#,
        "users",
    );
}

#[test]
fn http_delete_route() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
fn DeleteUser() -> Str => "deleted";
fn main() {
    let app = Server::new(":3000");
    app.delete("/users/:id", DeleteUser);
    app.start();
}
"#,
        "users",
    );
}

#[test]
fn http_patch_route() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
fn PatchUser() -> Str => "patched";
fn main() {
    let app = Server::new(":3000");
    app.patch("/users/:id", PatchUser);
    app.start();
}
"#,
        "users",
    );
}

#[test]
fn http_multiple_routes() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
fn ListUsers() -> Str => "users";
fn GetUser() -> Str => "user";
fn main() {
    let app = Server::new(":3000");
    app.get("/users", ListUsers);
    app.get("/users/:id", GetUser);
    app.start();
}
"#,
        "users",
    );
}

// =============================================================================
// 4. PATH PARAMETERS
// =============================================================================

#[test]
fn http_path_params_struct() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
struct UserParams { id: Int }
fn GetUser(params: UserParams) -> Str => "user ${params.id}";
fn main() {
    let app = Server::new(":3000");
    app.get("/users/:id", GetUser);
    app.start();
}
"#,
        "id",
    );
}

#[test]
fn http_multiple_path_params() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
struct PostParams { userId: Int, postId: Int }
fn GetPost(params: PostParams) -> Str => "post ${params.postId} of user ${params.userId}";
fn main() {
    let app = Server::new(":3000");
    app.get("/users/:userId/posts/:postId", GetPost);
    app.start();
}
"#,
        "postId",
    );
}

// =============================================================================
// 5. QUERY PARAMETERS
// =============================================================================

#[test]
fn http_query_params() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
struct SearchQuery { q: Str, page: Int }
fn Search(query: SearchQuery) -> Str => "searching ${query.q} page ${query.page}";
fn main() {
    let app = Server::new(":3000");
    app.get("/search", Search);
    app.start();
}
"#,
        "search",
    );
}

// =============================================================================
// 6. REQUEST BODY (POST/PUT)
// =============================================================================

#[test]
fn http_body_parsing() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
struct CreateUserBody { name: Str, email: Str }
fn CreateUser(body: CreateUserBody) -> Str => "created ${body.name}";
fn main() {
    let app = Server::new(":3000");
    app.post("/users", CreateUser);
    app.start();
}
"#,
        "CreateUserBody",
    );
}

#[test]
fn http_body_nested_struct() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
struct Address { city: Str }
struct CreateUserBody { name: Str, address: Address }
fn CreateUser(body: CreateUserBody) -> Str => "created ${body.name} in ${body.address.city}";
fn main() {
    let app = Server::new(":3000");
    app.post("/users", CreateUser);
    app.start();
}
"#,
        "city",
    );
}

// =============================================================================
// 7. MIDDLEWARE — CORS
// =============================================================================

#[test]
fn http_cors_default() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
fn Ping() -> Str => "pong";
fn main() {
    let app = Server::new(":3000");
    app.cors();
    app.get("/ping", Ping);
    app.start();
}
"#,
        "cors",
    );
}

// =============================================================================
// 8. MIDDLEWARE — RATE LIMITING
// =============================================================================

#[test]
fn http_ratelimit_default() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
fn Ping() -> Str => "pong";
fn main() {
    let app = Server::new(":3000");
    app.ratelimit();
    app.get("/ping", Ping);
    app.start();
}
"#,
        "ratelimit",
    );
}

// =============================================================================
// 9. ROUTE GROUPS
// =============================================================================

#[test]
fn http_route_group() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
fn ListUsers() -> Str => "users";
fn GetUser() -> Str => "user";
fn main() {
    let app = Server::new(":3000");
    app.group("/api", {
        get("/users", ListUsers),
        get("/users/:id", GetUser),
    });
    app.start();
}
"#,
        "api",
    );
}

#[test]
fn http_nested_route_groups() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
fn ListPosts() -> Str => "posts";
fn main() {
    let app = Server::new(":3000");
    app.group("/api/v1", {
        get("/posts", ListPosts),
    });
    app.start();
}
"#,
        "api/v1",
    );
}

// =============================================================================
// 10. CRUD AUTO-GENERATION
// =============================================================================

#[test]
fn http_crud_basic() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
import std::Database;
struct User {
    id: Int @primary @auto,
    name: Str,
    email: Str @unique,
}
fn main() {
    let db = Database::Postgres()?;
    let app = Server::new(":3000");
    app.crud("/users", User, db);
    app.start();
}
"#,
        "crud",
    );
}

#[test]
fn http_crud_with_decorators() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
import std::Database;
struct User {
    id: Int @primary @auto,
    Email: Str @email @unique,
    Password: Str @hash @writeOnly,
    Name: Str,
}
fn main() {
    let db = Database::Postgres()?;
    let app = Server::new(":3000");
    app.crud("/users", User, db);
    app.start();
}
"#,
        "crud",
    );
}

// =============================================================================
// 11. CUSTOM MIDDLEWARE
// =============================================================================

#[test]
fn http_custom_middleware_on_route() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server, Request, Response, Next};
enum AuthError { Unauthorized }
fn AuthMiddleware(req: Request, next: Next) -> Response ! AuthError {
    let token = req.header("Authorization");
    if token == nil { return Err AuthError::Unauthorized; }
    let response = next.call();
    return Ok response;
}
fn GetProfile() -> Str => "profile";
fn main() {
    let app = Server::new(":3000");
    app.get("/profile", AuthMiddleware, GetProfile);
    app.start();
}
"#,
        "Authorization",
    );
}

#[test]
fn http_global_middleware() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server, Request, Response, Next};
fn LogMiddleware(req: Request, next: Next) -> Response ! Str {
    print("Request received");
    let response = next.call();
    return Ok response;
}
fn Ping() -> Str => "pong";
fn main() {
    let app = Server::new(":3000");
    app.get("/ping", LogMiddleware, Ping);
    app.start();
}
"#,
        "Request received",
    );
}

#[test]
fn http_multiple_middleware_stacked() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server, Request, Response, Next};
enum AuthError { Unauthorized }
fn AuthMiddleware(req: Request, next: Next) -> Response ! AuthError {
    Ok next.call();
}
fn AdminMiddleware(req: Request, next: Next) -> Response ! AuthError {
    Ok next.call();
}
fn GetAdmin() -> Str => "admin panel";
fn main() {
    let app = Server::new(":3000");
    app.get("/admin", AuthMiddleware, AdminMiddleware, GetAdmin);
    app.start();
}
"#,
        "admin panel",
    );
}

// =============================================================================
// 12. JWT MIDDLEWARE
// =============================================================================

#[test]
fn http_jwt_on_route() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
import std::Auth::{Jwt};
import std::Database;
struct User { id: Int @primary @auto }
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
fn http_jwt_on_group() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
import std::Auth::{Jwt};
import std::Database;
struct User { id: Int @primary @auto }
fn ListItems() -> Str => "items";
fn main() {
    let db = Database::Postgres()?;
    let app = Server::new(":3000");
    app.auth("/signup", "/login", User, db);
    app.group("/api", Jwt(), {
        get("/items", ListItems),
    });
    app.start();
}
"#,
        "api",
    );
}

// =============================================================================
// 13. HANDLER RETURN TYPES
// =============================================================================

#[test]
fn http_handler_returns_struct() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
struct UserResponse { name: Str, email: Str }
fn GetUser() -> UserResponse {
    return UserResponse { name: "Alice", email: "a@x.com" };
}
fn main() {
    let app = Server::new(":3000");
    app.get("/user", GetUser);
    app.start();
}
"#,
        "Alice",
    );
}

#[test]
fn http_handler_returns_array() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
struct User { name: Str }
fn ListUsers() -> [User] {
    return [User { name: "Alice" }, User { name: "Bob" }];
}
fn main() {
    let app = Server::new(":3000");
    app.get("/users", ListUsers);
    app.start();
}
"#,
        "Bob",
    );
}

#[test]
fn http_handler_returns_error() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
fn GetUser() -> Str ! Str {
    Err "not found";
}
fn main() {
    let app = Server::new(":3000");
    app.get("/user", GetUser);
    app.start();
}
"#,
        "not found",
    );
}

// =============================================================================
// 14. COMBINED COMPLEX SCENARIOS
// =============================================================================

#[test]
fn http_full_api_pattern() {
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

enum AuthError { Unauthorized }

fn AuthMiddleware(req: Request, next: Next) -> Response ! AuthError {
    let token = req.header("Authorization");
    if token == nil { return Err AuthError::Unauthorized; }
    let response = next.call();
    return Ok response;
}

fn Ping() -> Str => "pong";
fn GetProfile() -> Str => "profile";

fn main() {
    let db = Database::Postgres()?;
    let app = Server::new(":3000");
    app.cors();

    app.auth("/signup", "/login", User, db);
    app.get("/ping", Ping);

    app.group("/api", Jwt(), {
        get("/profile", GetProfile),
    });

    app.crud("/admin/users", User, db);
    app.start();
}
"#,
        "admin/users",
    );
}

// =============================================================================
// 15. EDGE CASES
// =============================================================================

#[test]
fn http_empty_route_path() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
fn Root() -> Str => "root";
fn main() {
    let app = Server::new(":3000");
    app.get("/", Root);
    app.start();
}
"#,
        "root",
    );
}

#[test]
fn http_deeply_nested_groups() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
fn Deep() -> Str => "deep";
fn main() {
    let app = Server::new(":3000");
    app.group("/a/b/c/d", {
        get("/deep", Deep),
    });
    app.start();
}
"#,
        "deep",
    );
}

#[test]
fn http_handler_with_all_param_types() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
struct PathP { id: Int }
struct QueryP { fields: Str }
struct BodyP { name: Str, email: Str }
fn UpdateUser(params: PathP, query: QueryP, body: BodyP) -> Str {
    return "updated ${params.id}";
}
fn main() {
    let app = Server::new(":3000");
    app.put("/users/:id", UpdateUser);
    app.start();
}
"#,
        "UpdateUser",
    );
}

#[test]
fn http_optional_query_params() {
    assert_ffi_compiles_with(
        r#"
import std::Http::{Server};
struct SearchQuery { q: Str, page: Int?, limit: Int? }
fn Search(query: SearchQuery) -> Str {
    let p = query.page ?? panic("no page");
    let l = query.limit ?? panic("no limit");
    return "search ${query.q} page ${p} limit ${l}";
}
fn main() {
    let app = Server::new(":3000");
    app.get("/search", Search);
    app.start();
}
"#,
        "search",
    );
}
