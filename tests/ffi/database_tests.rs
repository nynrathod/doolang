//! Database FFI Tests — Production-grade coverage
//! Full compiler pipeline: lex → parse → analyze → MIR → codegen
//! Syntax matches dev_test/database/: import std::Database, Database::Postgres()

use super::{assert_ffi_compiles, assert_ffi_compiles_with};

// =============================================================================
// 1. DATABASE CONNECTION
// =============================================================================

#[test]
fn db_postgres_connect() {
    assert_ffi_compiles(
        r#"
import std::Database;
fn main() {
    let db = Database::Postgres()?;
}
"#,
    );
}

#[test]
fn db_postgres_connect_with_url() {
    assert_ffi_compiles_with(
        r#"
import std::Database;
fn main() {
    let db = Database::Postgres("postgres://user:pass@localhost:5432/mydb")?;
}
"#,
        "mydb",
    );
}

// =============================================================================
// 2. TABLE DEFINITION via Structs + Decorators
// =============================================================================

#[test]
fn db_struct_primary_auto() {
    assert_ffi_compiles(
        r#"
import std::Database;
struct User {
    id: Int @primary @auto,
    name: Str,
}
fn main() { let db = Database::Postgres()?; }
"#,
    );
}

#[test]
fn db_struct_all_decorators() {
    assert_ffi_compiles(
        r#"
import std::Database;
struct User {
    id: Int @primary @auto,
    Email: Str @email @unique,
    Password: Str @hash @writeOnly,
    Name: Str,
    Credits: Int @readOnly @default(100),
    Role: Str @default("user"),
    InternalId: Str @internal,
    bio: Str?,
}
fn main() { let db = Database::Postgres()?; }
"#,
    );
}

#[test]
fn db_struct_optional_fields() {
    assert_ffi_compiles(
        r#"
import std::Database;
struct Article {
    id: Int @primary @auto,
    title: Str,
    content: Str,
    publishedAt: Str?,
    tags: [Str]?,
}
fn main() { let db = Database::Postgres()?; }
"#,
    );
}

// =============================================================================
// 3. CRUD OPERATIONS
// =============================================================================

#[test]
fn db_insert_record() {
    assert_ffi_compiles_with(
        r#"
import std::Database;
struct Todo {
    id: Int @primary @auto,
    title: Str,
    done: Bool @default(false),
}
fn main() {
    let mut db = Database::Postgres()?;
    let todo = Todo { id: 0, title: "Buy milk", done: false };
    db.insert(todo)?;
}
"#,
        "Buy milk",
    );
}

#[test]
fn db_find_by_id() {
    assert_ffi_compiles(
        r#"
import std::Database;
struct Todo {
    id: Int @primary @auto,
    title: Str,
    done: Bool,
}
fn main() {
    let mut db = Database::Postgres()?;
    let todo = db.findById(Todo, 1)?;
    print(todo.title);
}
"#,
    );
}

#[test]
fn db_find_all() {
    assert_ffi_compiles(
        r#"
import std::Database;
struct Todo {
    id: Int @primary @auto,
    title: Str,
    done: Bool,
}
fn main() {
    let mut db = Database::Postgres()?;
    let todos = db.findAll(Todo)?;
    for todo in todos {
        print(todo.title);
    }
}
"#,
    );
}

#[test]
fn db_update_record() {
    assert_ffi_compiles(
        r#"
import std::Database;
struct Todo {
    id: Int @primary @auto,
    title: Str,
    done: Bool,
}
fn main() {
    let mut db = Database::Postgres()?;
    let mut todo = db.findById(Todo, 1)?;
    todo.done = true;
    db.update(todo)?;
}
"#,
    );
}

#[test]
fn db_delete_record() {
    assert_ffi_compiles(
        r#"
import std::Database;
struct Todo {
    id: Int @primary @auto,
    title: Str,
    done: Bool,
}
fn main() {
    let mut db = Database::Postgres()?;
    db.deleteById(Todo, 1)?;
}
"#,
    );
}

// =============================================================================
// 4. RAW QUERIES
// =============================================================================

#[test]
fn db_raw_query() {
    assert_ffi_compiles_with(
        r#"
import std::Database;
fn main() {
    let mut db = Database::Postgres()?;
    let results = db.raw("SELECT * FROM users WHERE age > 18")?;
    print(results);
}
"#,
        "SELECT * FROM",
    );
}

#[test]
fn db_raw_query_with_params() {
    assert_ffi_compiles_with(
        r#"
import std::Database;
fn main() {
    let mut db = Database::Postgres()?;
    let results = db.rawWithParams("SELECT * FROM users WHERE city = $1", ["NYC"])?;
    print(results);
}
"#,
        "NYC",
    );
}

// =============================================================================
// 5. ERROR HANDLING for DB Operations
// =============================================================================

#[test]
fn db_error_handling_insert() {
    assert_ffi_compiles_with(
        r#"
import std::Database;
struct User {
    id: Int @primary @auto,
    email: Str @unique,
    name: Str,
}
fn createUser(db: Database, name: Str, email: Str) -> User ! Str {
    let mut d = db;
    let user = User { id: 0, name: name, email: email };
    let result, err = d.insert(user);
    if err != nil {
        Err "Failed to create user: duplicate email";
    }
    Ok result;
}
fn main() {
    let db = Database::Postgres()?;
}
"#,
        "duplicate email",
    );
}

#[test]
fn db_error_handling_find() {
    assert_ffi_compiles_with(
        r#"
import std::Database;
struct User {
    id: Int @primary @auto,
    name: Str,
}
fn main() {
    let mut db = Database::Postgres()?;
    let user, err = db.findById(User, 1);
    if err != nil {
        print("User not found");
    } else {
        print(user.name);
    }
}
"#,
        "User not found",
    );
}

#[test]
fn db_error_propagation_chain() {
    assert_ffi_compiles_with(
        r#"
import std::Database;
struct User {
    id: Int @primary @auto,
    name: Str,
}
fn main() {
    let mut db = Database::Postgres()?;
    let user = db.findById(User, 1)?;
    let userName = user.name;
    print(userName);
}
"#,
        "call",
    );
}

// =============================================================================
// 6. COMPLEX DB SCENARIOS
// =============================================================================

#[test]
fn db_multi_table() {
    assert_ffi_compiles_with(
        r#"
import std::Database;
struct User {
    id: Int @primary @auto,
    name: Str,
    email: Str @unique,
}
struct Post {
    id: Int @primary @auto,
    title: Str,
    content: Str,
    authorId: Int,
}
fn main() {
    let mut db = Database::Postgres()?;
    let user = User { id: 0, name: "Alice", email: "a@x.com" };
    db.insert(user)?;
    let post = Post { id: 0, title: "Hello", content: "World", authorId: 1 };
    db.insert(post)?;
}
"#,
        "Alice",
    );
}

#[test]
fn db_struct_with_enum_field() {
    assert_ffi_compiles_with(
        r#"
import std::Database;
enum Status { Active, Inactive, Pending }
struct User {
    id: Int @primary @auto,
    name: Str,
    status: Status @default(Status::Active),
}
fn main() {
    let mut db = Database::Postgres()?;
    let user = User { id: 0, name: "Alice", status: Status::Active };
    db.insert(user)?;
}
"#,
        "Alice",
    );
}
