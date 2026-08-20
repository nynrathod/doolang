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

# Install doo compiler (latest)
RUN DOO_TAG=$(curl -fsSL https://api.github.com/repos/nynrathod/doolang/releases/latest | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/') && \
    DOO_VERSION=${DOO_TAG#v} && \
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
ENV DOO_ENV=production
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
DATABASE_URL=postgresql://postgres:YOUR_PASSWORD_HERE@localhost:5432/doo_test

# Authentication
JWT_SECRET=your-secret-key-change-in-production
"#;

// Shared render.yaml content
pub const RENDER_YAML_CONTENT: &str = r#"services:
  - type: web
    name: doo-app
    env: docker
    plan: free
    region: ohio
    envVars:
      - key: DOO_ENV
        value: production
      - key: DATABASE_URL
        sync: false
      - key: JWT_SECRET
        sync: false
"#;

pub const STARTER_TEMPLATE: Template = Template {
    name: "starter",
    description: "Hello World API",
    files: &[
        TemplateFile {
            path: "main.doo",
            content: r#"// 🔥 Doo - The fastest way to build and deploy production APIs
// Template: Starter
// Run: doo run | Deploy: doo deploy
// Learn more at: https://github.com/nynrathod/doolang

import std::Http::Server;

// Handlers are automatically serialized to JSON by Doo
fn home() -> Str {
    return "Hello World from Doo!";
}

fn main() {
    let app = Server::new(":3000");

    // Register a GET route at "/" that calls the home() function
    app.get("/", home);

    // Start the server
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
            content: r#"// 🔥 Doo - The fastest way to build and deploy production APIs
// Template: Todo API (jwt auth & crud)
// Run: doo run | Deploy: doo deploy
// Learn more at: https://github.com/nynrathod/doolang

import std::Http::Server;
import std::Database;
import std::Auth::Jwt;

struct User {
    id: Int @primary @auto,                 // Primary key, auto-incremented
    Email: Str @email @unique,              // Email validation + unique constraint
    Password: Str @hash @min(8) @max(20),   // Auto-hashed, length validated
    Name: Str,
}


struct Todo {
    id: Int @primary @auto,
    Title: Str,
    Completed: Bool @default(false),
    UserId: Int @foreign(User),
}

static DB: Database;

fn main() {
    // Auto read DATABASE_URL in .env
    DB = Database::Postgres()?;

    let app = Server::new(":3105");

    // Authentication via JWT
    app.auth("/signup", "/login", User, DB);

    // Todos:    GET, POST, GET/:id, PUT/:id, DELETE/:id at /todos
    app.crud("/todos", Todo, DB);

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
    id: Int @primary @auto,                 // Primary key, auto-incremented
    Email: Str @email @unique,              // Email validation + unique constraint
    Password: Str @hash @min(8) @max(20),   // Auto-hashed, length validated
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

fn GetFeed() -> [Post] ! DatabaseError {

    let result: [Post] = DB.raw("
        SELECT p.title
        FROM posts p
        JOIN users u ON p.author_id = u.id
        WHERE p.published = true
    ")?;

    Ok result;
}

fn GetUserPosts(authorId: Int) -> [Post] ! DatabaseError {
    let result: [Post] = DB.rawWithParams("
        SELECT * FROM posts
        WHERE author_id = $1
    ", authorId)?;

    Ok result;
}

fn GetMyPosts(userId: Int) -> [Post] ! DatabaseError {
    let result: [Post] = DB.rawWithParams("
        SELECT * FROM posts
        WHERE author_id = $1
    ", userId)?;

    Ok result;
}
"#,
        },
        TemplateFile {
            path: "main.doo",
            content: r#"// 🔥 Doo - The fastest way to build and deploy production APIs
// Template: Blog API (Posts + Comments + JWT Auth)
// Run: doo run | Deploy: doo deploy
// Learn more at: https://github.com/nynrathod/doolang

import std::Http::Server;
import std::Database;
import std::Auth::Jwt;
import models::{Post, Comment, User};
import handlers::{GetFeed, GetUserPosts, GetMyPosts};

static DB: Database;

fn main() {
    // Auto read DATABASE_URL in .env
    DB = Database::Postgres()?;

    let app = Server::new(":3106");

    // Authentication via JWT
    app.auth("/api/auth/signup", "/api/auth/login", User, DB);

    // Posts:    GET, POST, GET/:id, PUT/:id, DELETE/:id at /api/posts
    // Comments: GET, POST, GET/:id, PUT/:id, DELETE/:id at /api/comments
    app.crud("/api/posts", Post, DB);
    app.crud("/api/comments", Comment, DB);

    app.get("/api/feed",  GetFeed);
    app.get("/api/user/:authorId/posts", GetUserPosts);
    app.get("/api/user/posts", Jwt(), GetMyPosts);
    app.get("/api/public/feed",  GetFeed);

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
