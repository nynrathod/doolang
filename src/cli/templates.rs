pub struct TemplateFile {
    pub path: &'static str,
    pub content: &'static str,
}

pub struct Template {
    pub name: &'static str,
    pub description: &'static str,
    pub files: &'static [TemplateFile],
}

// Shared Dockerfile content for all templates
pub const DOCKERFILE_CONTENT: &str = r#"# Doo application Dockerfile
# Multi-stage build for minimal image size

# Build stage
FROM debian:bookworm-slim AS builder

RUN apt-get update && apt-get install -y \
    curl clang build-essential libssl-dev pkg-config unzip \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .

# Install doo compiler (prerelease)
RUN DOO_TAG=v0.3.0-pre && \
    DOO_VERSION=0.3.0 && \
    mkdir -p ~/.doo/bin ~/.doo/bin/std && \
    curl -fsSL \
      https://github.com/nynrathod/doolang/releases/download/${DOO_TAG}/doo-linux-${DOO_VERSION}.zip \
      -o /tmp/doo.zip && \
    unzip -q /tmp/doo.zip -d /tmp/doo-extracted && \
    EXTRACT_DIR=$(find /tmp/doo-extracted -mindepth 1 -maxdepth 1 -type d | head -1) && \
    cp "$EXTRACT_DIR/doo" ~/.doo/bin/doo && \
    (cp "$EXTRACT_DIR"/*.so ~/.doo/bin/ 2>/dev/null || true) && \
    cp -r "$EXTRACT_DIR"/std/* ~/.doo/bin/std/ && \
    chmod +x ~/.doo/bin/doo && \
    rm -rf /tmp/doo.zip /tmp/doo-extracted

RUN ~/.doo/bin/doo build -o app

# Runtime stage
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*
WORKDIR /app
RUN mkdir -p /app/lib
COPY --from=builder /app/app .
COPY --from=builder /root/.doo/bin/*.so /app/lib/
ENV LD_LIBRARY_PATH=/app/lib:/app
EXPOSE 3000
CMD ["./app"]
"#;

// Shared .gitignore content for all templates
pub const GITIGNORE_CONTENT: &str = r#"# Doo build artifacts
*.o
*.ll
output
app

# Environment
.env.local
.env.*.local

# IDE
.idea/
.vscode/
*.swp
"#;

// Shared .env content for all templates
pub const ENV_CONTENT: &str = r#"# Database connection
DATABASE_URL=postgresql://postgres:admin@localhost:5432/doo_test

# Authentication
JWT_SECRET=your-secret-key-change-in-production
"#;

pub const STARTER_TEMPLATE: Template = Template {
    name: "starter",
    description: "Hello World API",
    files: &[
        TemplateFile {
            path: "main.doo",
            content: r#"import std::Http::Server;

fn home() -> Str {
    return "Hello World from Doo!";
}

fn main() {
    let app = Server::new(":3000");

    app.get("/", home);

    app.start();
}
"#,
        },
        TemplateFile {
            path: ".env",
            content: ENV_CONTENT,
        },
        TemplateFile {
            path: ".gitignore",
            content: GITIGNORE_CONTENT,
        },
        TemplateFile {
            path: "Dockerfile",
            content: DOCKERFILE_CONTENT,
        },
    ],
};

pub const TODO_TEMPLATE: Template = Template {
    name: "todo",
    description: "Auth + CRUD",
    files: &[
        TemplateFile {
            path: "main.doo",
            content: r#"import std::Http::Server;
import std::Database;
import std::Auth::jwt;

struct User {
    id: Int @primary @auto,
    Email: Str @email @unique,
    Password: Str @hash @min(8) @max(20),
    Name: Str,
}

struct Todo {
    id: Int @primary @auto,
    Title: Str,
    Completed: Bool @default(false),
    UserId: Int, // Link to User
}

fn main() {
    // Auto-propagate error if connection fails
    let db = Database::postgres()?;
    let app = Server::new(":3000");

    // Generates:
    // - POST /signup (email, password, name)
    // - POST /login (email, password) -> Returns JWT
    app.auth("/signup", "/login", User, db);

    // Generates:
    // - GET    /todos       (List)
    // - POST   /todos       (Create)
    // - GET    /todos/:id   (Get one)
    // - PUT    /todos/:id   (Update)
    // - DELETE /todos/:id   (Delete)
    app.crud("/todos", Todo, db);

    app.start();
}
"#,
        },
        TemplateFile {
            path: ".env",
            content: ENV_CONTENT,
        },
        TemplateFile {
            path: ".gitignore",
            content: GITIGNORE_CONTENT,
        },
        TemplateFile {
            path: "Dockerfile",
            content: DOCKERFILE_CONTENT,
        },
    ],
};

pub const BLOG_TEMPLATE: Template = Template {
    name: "blog",
    description: "Multi-file + Relations",
    files: &[
        TemplateFile {
            path: "models.doo",
            content: r#"struct User {
    id: Int @primary @auto,
    Email: Str @email @unique,
    Password: Str @hash @min(8) @max(20),
    Name: Str,
    Role: Str @default("user"),
}

struct Post {
    id: Int @primary @auto,
    Title: Str,
    Content: Str,
    Published: Bool @default(false),
    AuthorId: Int,
}

struct Comment {
    id: Int @primary @auto,
    Content: Str,
    PostId: Int,
    AuthorId: Int,
}
"#,
        },
        TemplateFile {
            path: "handlers.doo",
            content: r#"import std::Database::{DatabaseError};
import models::{Post};

// Custom handler using raw SQL with global database
fn GetFeed() -> [Post] ! DatabaseError {
    // Access global database instance
    let db = Database::get()?;

    // db.raw returns a JSON string of the result
    let result: [Post] = db.raw("
        SELECT p.title, u.name as authorName
        FROM Post p
        JOIN User u ON p.AuthorId = u.id
        WHERE p.Published = true
    ");

    Ok result;
}

// Get posts for a specific user by ID
fn GetUserPosts(authorId: Int) -> [Post] ! DatabaseError {
    let db = Database::get()?;

    let result: [Post] = db.rawWithParams("
        SELECT * FROM Post
        WHERE AuthorId = $1
    ", authorId);

    Ok result;
}
"#,
        },
        TemplateFile {
            path: "main.doo",
            content: r#"
                    import std::Http::Server;
import std::Database;
import std::Auth::jwt;
import models::{Post, Comment, User};
import handlers::{GetFeed, GetUserPosts};

fn main() {
    let db = Database::postgres()?;
    let app = Server::new(":3000");

    app.auth("/api/auth/signup", "/api/auth/login", User, db);

    app.crud("/api/posts", Post, db);
    app.crud("/api/comments", Comment, db);

    app.get("/api/feed",  GetFeed);
    app.get("/api/user/posts", GetUserPosts);


    app.get("/api/public/feed",  GetFeed);
    // app.get("/api/user/posts", GetUserPosts);

    app.start();
}
                "#,
        },
        TemplateFile {
            path: ".env",
            content: ENV_CONTENT,
        },
        TemplateFile {
            path: ".gitignore",
            content: GITIGNORE_CONTENT,
        },
        TemplateFile {
            path: "Dockerfile",
            content: DOCKERFILE_CONTENT,
        },
    ],
};

pub const TEMPLATES: &[Template] = &[STARTER_TEMPLATE, TODO_TEMPLATE, BLOG_TEMPLATE];

// ============================================================================
// ARCHIVED: Docker Deployment Template
// ============================================================================
// The following Dockerfile content is archived for reference when Docker
// deployment support is needed. Currently using native binary deployment.
//
// Working Dockerfile for Doo applications:
// ```dockerfile
// # Doo application Dockerfile
// # Multi-stage build for minimal image size
//
// # Build stage
// FROM debian:bookworm-slim AS builder
//
// RUN apt-get update && apt-get install -y \
//     curl clang build-essential libssl-dev pkg-config unzip \
//     && rm -rf /var/lib/apt/lists/*
//
// WORKDIR /app
// COPY . .
//
// # Install doo compiler (prerelease)
// RUN DOO_TAG=v0.3.0-pre && \
//     DOO_VERSION=0.3.0 && \
//     mkdir -p ~/.doo/bin ~/.doo/bin/std && \
//     curl -fsSL \
//       https://github.com/nynrathod/doolang/releases/download/${DOO_TAG}/doo-linux-${DOO_VERSION}.zip \
//       -o /tmp/doo.zip && \
//     unzip -q /tmp/doo.zip -d /tmp/doo-extracted && \
//     EXTRACT_DIR=$(find /tmp/doo-extracted -mindepth 1 -maxdepth 1 -type d | head -1) && \
//     cp "$EXTRACT_DIR/doo" ~/.doo/bin/doo && \
//     (cp "$EXTRACT_DIR"/*.so ~/.doo/bin/ 2>/dev/null || true) && \
//     cp -r "$EXTRACT_DIR"/std/* ~/.doo/bin/std/ && \
//     chmod +x ~/.doo/bin/doo && \
//     rm -rf /tmp/doo.zip /tmp/doo-extracted
//
// RUN ~/.doo/bin/doo build -o app
//
// # Runtime stage
// FROM debian:bookworm-slim
// RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*
// WORKDIR /app
// RUN mkdir -p /app/lib
// COPY --from=builder /app/app .
// COPY --from=builder /root/.doo/bin/*.so /app/lib/
// ENV LD_LIBRARY_PATH=/app/lib:/app
// EXPOSE 3000
// CMD ["./app"]
// ```
// ============================================================================
