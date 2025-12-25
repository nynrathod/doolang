# Doo - Ship Production APIs in Minutes, Not Days

[![Rust](https://img.shields.io/badge/Made%20with-Rust-orange)](https://www.rust-lang.org/)
[![LLVM](https://img.shields.io/badge/LLVM-blueviolet)](https://llvm.org/)
[![Native](https://img.shields.io/badge/Compiles%20to-Native-green)](https://llvm.org/)

Doo is a statically-typed, compiled programming language built in Rust + LLVM, designed for building production APIs quickly and safely. It uses automatic memory management via reference counting.

**Stop wrestling with boilerplate. Write type-safe APIs and deploy with one command.**

```rust
// main.doo - Your entire API
import std::Http::Server;
import std::Database;

struct User {
    id: Int @primary @auto,
    email: Str @email @unique,
    password: Str @hash,
}

fn main() {
    let db = Database::postgres()?;
    let app = Server::new(":3000");

    // Authentication via JWT
    app.auth("/signup", "/login", User, db);

    // Full CRUD
    // GET, POST, GET/:id, PUT/:id, DELETE/:id
    app.crud("/users", User, db);

    app.start();
}
```

**Run it:**

```bash
doo run  # Compiles to native + starts server
```

---

## Why Doo?

| Traditional Stack          | Doo                       |
| -------------------------- | ------------------------- |
| 500+ lines of boilerplate  | 15 lines of code          |
| 3 config files             | Zero config               |
| Manual validation          | Auto-validated decorators |
| Separate deployment setup  | One command deploy        |
| Type mismatches at runtime | Compile-time safety       |

---

## 🔧 Installation

### Windows (PowerShell)

```powershell
irm https://raw.githubusercontent.com/nynrathod/doolang/main/install.ps1 | iex
```

### Linux / macOS

```bash
curl -fsSL https://raw.githubusercontent.com/nynrathod/doolang/main/install.sh | bash
```

### Verify Installation

```bash
doo --help
```

---

## 🎯 Quick Start

### Starter Template

```bash
doo init --template starter starter-api
cd starter-api
doo run
```

### Blog Template

```bash
doo init --template blog blog-api  # Blog posts + comments API
cd blog-api
doo run
```

Visit `http://localhost:3000` - your API is live.

---

## 📖 Real-World Example

Complete task management API with auth, CRUD, and custom queries:

```rust
import std::Http::Server;
import std::Database;

enum Status { Todo, InProgress, Done }
enum Priority { Low, Medium, High }

struct User {
    id: Int @primary @auto,
    email: Str @email @unique,
    password: Str @hash @min(8),
}

struct Task {
    id: Int @primary @auto,
    title: Str @min(1) @max(200),
    status: Status @default(Status::Todo),
    priority: Priority @default(Priority::Medium),
    userId: Int @foreign(User),
}

fn GetUrgent() -> [Task] ! DatabaseError {
    let db = Database::get()?;
    let result: [Task] = db.rawWithParams(
        "SELECT * FROM tasks WHERE priority = $1 AND status != $2",
        [Priority::High, Status::Done]
    )?;
    Ok result;
}

fn main() {
    let db = Database::postgres()?;
    let app = Server::new(":3000");

    app.auth("/signup", "/login", User, db);
    app.crud("/tasks", Task, db);
    app.get("/tasks/urgent", GetUrgent);

    app.start();
}
```

**That's it.** 40 lines for a production-ready API with:

- ✓ User authentication with JWT
- ✓ Password hashing
- ✓ Struct validation
- ✓ Full CRUD operations
- ✓ Custom business logic
- ✓ Auto error propagation
- ✓ Auto migrate table on startup

---

## 🌐 Language Essentials

### Variables & Types

```rust
let name = "Alice";         // Type inferred
let age: Int = 25;          // Explicit
let mut count = 0;          // Mutable
```

| Type     | Example     |
| -------- | ----------- |
| `Int`    | `42`        |
| `Float`  | `3.14`      |
| `Str`    | `"hello"`   |
| `Bool`   | `true`      |
| `[T]`    | `[1, 2, 3]` |
| `{K: V}` | `{"a": 1}`  |

### Structs & Validation

```rust
struct User {
    id: Int @primary @auto,
    email: Str @email @unique,
    password: Str @hash @min(8) @max(20),
    age: Int @min(18),
}

// Automatically create table on startup
struct AuditLog @table {
    id: Int @primary @auto,
    action: Str
}
```

### Error Handling

```rust
fn divide(a: Int, b: Int) -> Int ! Str {
    if b == 0 { Err "division by zero"; }
    Ok a / b;
}

let result = divide(10, 2)?;  // Auto-propagate errors
```

### HTTP Routes

```rust
fn GetUser(id: Int) -> User ! DatabaseError {
    let db = Database::get()?;
    Ok db.query("SELECT * FROM users WHERE id = $1", id)?;
}

app.get("/users/:id", GetUser);
```

**Full language guide:** See [`examples/`](examples/) folder for complete tutorials

---

## 📦 Examples

- **[tasks_api](examples/tasks_api/)** - Complete CRUD + Auth + Custom queries
- **[calculator](examples/calculator/)** - Error handling patterns
- **[clean_code](examples/clean_code/)** - Service layers & validation

**More examples:** [examples/](examples/)

---

## 💬 Community & Support

- **GitHub Discussions**: Ask questions, share projects
- **Issues**: Bug reports and feature requests
- **Contributing**: We welcome PRs! See [CONTRIBUTING.md](CONTRIBUTING.md)

---

## What's Next

We're focused on adoption first. Next features will be driven by real developer needs:

- More cloud providers + zero-config hosting
- Multi-database support (MySQL, SQLite etc)
- WebSocket, concurrency, async and many more

**Want to influence the roadmap?** [Open a discussion →](https://github.com/nynrathod/doolang/discussions)

---

## 📜 License

This project is licensed under the MIT License

---

## 🙏 Acknowledgments

- **LLVM Project**: For the powerful backend infrastructure
- **Rust Community**: For inspiration and excellent tooling

---

**Ready to ship faster?** `doo init starter` ← Start here

> **Want to contribute?** See [CONTRIBUTING.md](CONTRIBUTING.md)  
> **For testing and development:** See [TEST.md](TEST.md)
