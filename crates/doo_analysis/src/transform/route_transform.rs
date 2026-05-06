//! Route Group DSL Transformation
//!
//! Transforms route group DSL syntax into individual route registrations.
//!
//! ## Example transformation:
//! ```doo
//! app.group("/api", {
//!     get("/profile", getProfile),
//!     post("/posts", createPost)
//! })
//! ```
//!
//! Becomes:
//! ```doo
//! app.get("/api/profile", getProfile);
//! app.post("/api/posts", createPost);
//! ```
//!
//! ## With middleware:
//! ```doo
//! app.group("/api", AuthMiddleware, LogMiddleware, {
//!     get("/profile", getProfile),
//! })
//! ```
//!
//! Becomes:
//! ```doo
//! app.getWithMiddleware("/api/profile", "AuthMiddleware,LogMiddleware", "getProfile");
//! ```

use doo_core::Span;
use doo_frontend::ast::{Expr, ExprKind, FunctionDecl, Item, Program, Stmt, StmtKind};
use std::sync::atomic::{AtomicUsize, Ordering};

// ============================================================================
// HTTP Package Constants (used by route transform analysis)
// ============================================================================
// These are HTTP-package-specific constants. They live here (not in doo_core)
// because the compiler core should not know about package-specific values.
// Route transform is an analysis pass that specifically handles HTTP DSL syntax.

/// JWT middleware function name (the Doo `Jwt()` function)
const DOO_JWT_FUNC_NAME: &str = "Jwt";
/// JWT middleware identifier
const MIDDLEWARE_JWT: &str = "Jwt";
/// Default auth signup path
const DEFAULT_AUTH_SIGNUP_PATH: &str = "/auth/register";
/// Default auth login path
const DEFAULT_AUTH_LOGIN_PATH: &str = "/auth/login";

/// Global counter for generating unique closure function names.
static CLOSURE_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Transform route group DSL syntax in a program.
///
/// This function walks all items and statements, expanding `app.group()` calls
/// into individual route registrations with prefixes prepended.
pub fn transform_route_groups(program: &mut Program) {
    let mut new_items = Vec::new();

    for item in &mut program.items {
        match item {
            Item::Function(f) => {
                transform_stmts(&mut f.body);
            }
            Item::Statement(stmt) => {
                let mut stmts = vec![stmt.clone()];
                transform_stmts(&mut stmts);
                // If expanded into multiple statements, add extras as new items
                if stmts.len() > 1 {
                    *stmt = stmts.remove(0);
                    for extra in stmts {
                        new_items.push(Item::Statement(extra));
                    }
                } else if stmts.len() == 1 {
                    *stmt = stmts.remove(0);
                }
            }
            _ => {}
        }
    }

    // Add any expanded items
    program.items.extend(new_items);
}

/// Transform statements, expanding route groups.
fn transform_stmts(stmts: &mut Vec<Stmt>) {
    let mut i = 0;
    while i < stmts.len() {
        // Check if this statement contains a route group to expand
        if let Some(expanded) = try_expand_stmt_route_group(&stmts[i]) {
            let expanded_len = expanded.len();
            stmts.splice(i..i + 1, expanded);
            i += expanded_len;
        } else {
            // Recursively transform nested statements
            transform_stmt_recursive(&mut stmts[i]);
            i += 1;
        }
    }
}

/// Try to expand a statement containing a route group.
fn try_expand_stmt_route_group(stmt: &Stmt) -> Option<Vec<Stmt>> {
    match &stmt.kind {
        StmtKind::Expr(expr) => {
            if let Some(expanded) = try_expand_route_group(expr) {
                return Some(
                    expanded
                        .into_iter()
                        .map(|e| Stmt::new(StmtKind::Expr(e), stmt.span))
                        .collect(),
                );
            }
        }
        StmtKind::Let {
            value,
            mutable,
            pattern,
            type_ann,
        } => {
            if let Some(expanded) = try_expand_route_group(value) {
                // For let with route group, this is unusual but handle it
                if expanded.len() == 1 {
                    return Some(vec![Stmt::new(
                        StmtKind::Let {
                            mutable: *mutable,
                            pattern: pattern.clone(),
                            type_ann: type_ann.clone(),
                            value: expanded.into_iter().next().unwrap(),
                        },
                        stmt.span,
                    )]);
                }
            }
        }
        _ => {}
    }
    None
}

/// Try to expand a route group method call.
///
/// Returns Some(vec![expanded_calls]) if this is a group() call,
/// None otherwise.
fn try_expand_route_group(expr: &Expr) -> Option<Vec<Expr>> {
    if let ExprKind::MethodCall {
        object,
        method,
        args,
    } = &expr.kind
    {
        if method != "group" {
            return None;
        }

        // Must have at least 2 arguments: prefix and block
        if args.len() < 2 {
            return None;
        }

        // Extract prefix (first argument)
        let prefix_expr = &args[0];

        // Extract middleware (all args between prefix and block)
        let last_idx = args.len() - 1;
        let middleware_args = &args[1..last_idx];

        // Extract the block (last argument)
        let block = &args[last_idx];

        // Extract route definitions from the block
        let route_calls = match &block.kind {
            ExprKind::Block(stmts, result) => {
                let mut calls = extract_route_calls_from_stmts(stmts);
                if let Some(result_expr) = result {
                    if let Some(call) = extract_route_call(result_expr) {
                        calls.push(call);
                    }
                }
                calls
            }
            ExprKind::RouteBlock { routes } => {
                // Handle the new RouteBlock syntax: { get(...), post(...) }
                routes
                    .iter()
                    .filter_map(|r| extract_route_call(r))
                    .collect()
            }
            _ => {
                if let Some(call) = extract_route_call(block) {
                    vec![call]
                } else {
                    vec![]
                }
            }
        };

        // Transform each route call
        let expanded: Vec<Expr> = route_calls
            .into_iter()
            .map(|(http_method, path_expr, handler_expr)| {
                // Create full path: prefix + path
                let full_path = create_path_concat(prefix_expr, &path_expr, expr.span);

                // Convert path params :param -> {param}
                let full_path = convert_path_params_in_expr(full_path);

                // Convert handler to string
                let handler_str = convert_handler_to_string(handler_expr, expr.span);

                if middleware_args.is_empty() {
                    // No middleware: app.get(path, handler)
                    Expr::new(
                        ExprKind::MethodCall {
                            object: object.clone(),
                            method: http_method,
                            args: vec![full_path, handler_str],
                        },
                        expr.span,
                    )
                } else {
                    // With middleware: app.getWithMiddleware(path, "mw1,mw2", handler)
                    let middleware_names: Vec<String> = middleware_args
                        .iter()
                        .map(extract_identifier_name)
                        .collect();
                    let middleware_str = middleware_names.join(",");

                    Expr::new(
                        ExprKind::MethodCall {
                            object: object.clone(),
                            method: format!("{}WithMiddleware", http_method),
                            args: vec![
                                full_path,
                                Expr::new(ExprKind::StrLit(middleware_str), expr.span),
                                handler_str,
                            ],
                        },
                        expr.span,
                    )
                }
            })
            .collect();

        if !expanded.is_empty() {
            return Some(expanded);
        }
    }

    None
}

/// Extract route calls from statements.
fn extract_route_calls_from_stmts(stmts: &[Stmt]) -> Vec<(String, Expr, Expr)> {
    stmts
        .iter()
        .filter_map(|stmt| {
            if let StmtKind::Expr(e) = &stmt.kind {
                extract_route_call(e)
            } else {
                None
            }
        })
        .collect()
}

/// Extract a route call (method, path, handler) from an expression.
fn extract_route_call(expr: &Expr) -> Option<(String, Expr, Expr)> {
    if let ExprKind::Call { func, args } = &expr.kind {
        if let ExprKind::Ident(method_name) = &func.kind {
            if !is_http_method(method_name) {
                return None;
            }

            if args.len() != 2 {
                return None;
            }

            return Some((method_name.clone(), args[0].clone(), args[1].clone()));
        }
    }
    None
}

/// Check if a string is a valid HTTP method name.
fn is_http_method(name: &str) -> bool {
    matches!(
        name,
        "get" | "post" | "put" | "delete" | "patch" | "options" | "head"
    )
}

/// Check if a method name is a route registration method.
fn is_route_registration_method(name: &str) -> bool {
    matches!(
        name,
        "get" | "post" | "put" | "delete" | "patch" | "group" | "use" | "options" | "head"
    )
}

/// Create path concatenation: prefix + path.
fn create_path_concat(prefix: &Expr, path: &Expr, span: Span) -> Expr {
    // If both are string literals, optimize by concatenating at compile time
    if let (ExprKind::StrLit(prefix_str), ExprKind::StrLit(path_str)) = (&prefix.kind, &path.kind) {
        // Special case: if path is "/", don't add trailing slash
        if path_str == "/" {
            return Expr::new(ExprKind::StrLit(prefix_str.clone()), span);
        }

        // Ensure proper path joining (no double slashes)
        let full_path = if path_str.starts_with('/') {
            format!("{}{}", prefix_str.trim_end_matches('/'), path_str)
        } else {
            format!("{}/{}", prefix_str.trim_end_matches('/'), path_str)
        };
        return Expr::new(ExprKind::StrLit(full_path), span);
    }

    // Otherwise, create runtime concatenation
    Expr::new(
        ExprKind::Binary {
            left: Box::new(prefix.clone()),
            op: doo_frontend::ast::BinaryOp::Add,
            right: Box::new(path.clone()),
        },
        span,
    )
}

/// Convert handler expression to string literal.
/// NOTE: Keeping handler as-is for function pointer passing.
/// The codegen will pass the function pointer, not a string.
fn convert_handler_to_string(handler: Expr, _span: Span) -> Expr {
    // Keep the handler expression as-is - the codegen handles function pointers
    handler
}

/// Extract identifier name as String.
fn extract_identifier_name(expr: &Expr) -> String {
    match &expr.kind {
        ExprKind::Ident(name) => name.clone(),
        ExprKind::StrLit(s) => s.clone(),
        ExprKind::Call { func, args } => {
            if let ExprKind::Ident(func_name) = &func.kind {
                // Jwt() middleware function - returns middleware name constant
                if func_name == DOO_JWT_FUNC_NAME && args.is_empty() {
                    return MIDDLEWARE_JWT.to_string();
                }
            }
            "unknown".to_string()
        }
        _ => "unknown".to_string(),
    }
}

/// Convert path parameter syntax from :param to {param}.
fn convert_path_params_in_expr(expr: Expr) -> Expr {
    if let ExprKind::StrLit(path) = &expr.kind {
        let converted = convert_path_params(&path);
        Expr::new(ExprKind::StrLit(converted), expr.span)
    } else {
        expr
    }
}

/// Convert path parameter syntax from :param to {param}.
fn convert_path_params(path: &str) -> String {
    let mut result = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == ':' {
            result.push('{');

            while let Some(&next_ch) = chars.peek() {
                if next_ch.is_alphanumeric() || next_ch == '_' {
                    result.push(chars.next().unwrap());
                } else {
                    break;
                }
            }

            result.push('}');
        } else {
            result.push(ch);
        }
    }

    result
}

/// Transform inline closures in route registrations into named functions.
///
/// Finds patterns like:
///   app.post("/path", (params) -> ReturnType { body })
///
/// And transforms to:
///   fn __closure_path_N(params) -> ReturnType { body }
///   app.post("/path", "__closure_path_N")
pub fn transform_inline_closures(program: &mut Program) {
    let mut new_functions = Vec::new();

    for item in &mut program.items {
        match item {
            Item::Function(f) => {
                extract_closures_from_stmts(&mut f.body, &mut new_functions);
            }
            Item::Statement(stmt) => {
                extract_closures_from_stmt(stmt, &mut new_functions);
            }
            _ => {}
        }
    }

    // Insert generated functions at the beginning
    for func in new_functions.into_iter().rev() {
        program.items.insert(0, Item::Function(func));
    }
}

/// Extract closures from statements.
fn extract_closures_from_stmts(stmts: &mut [Stmt], generated: &mut Vec<FunctionDecl>) {
    for stmt in stmts {
        extract_closures_from_stmt(stmt, generated);
    }
}

/// Extract closures from a single statement.
fn extract_closures_from_stmt(stmt: &mut Stmt, generated: &mut Vec<FunctionDecl>) {
    match &mut stmt.kind {
        StmtKind::Expr(expr) => {
            extract_closures_from_expr(expr, generated);
        }
        StmtKind::Let { value, .. } => {
            extract_closures_from_expr(value, generated);
        }
        StmtKind::If {
            condition,
            then_block,
            else_branch,
        } => {
            extract_closures_from_expr(condition, generated);
            extract_closures_from_stmts(then_block, generated);
            if let Some(else_b) = else_branch {
                match else_b {
                    doo_frontend::ast::ElseBranch::Block(stmts) => {
                        extract_closures_from_stmts(stmts, generated);
                    }
                    doo_frontend::ast::ElseBranch::ElseIf(s) => {
                        extract_closures_from_stmt(s, generated);
                    }
                }
            }
        }
        StmtKind::For { body, iterable, .. } => {
            if let Some(iter) = iterable {
                extract_closures_from_expr(iter, generated);
            }
            extract_closures_from_stmts(body, generated);
        }
        StmtKind::Return(exprs) | StmtKind::Print(exprs) => {
            for e in exprs {
                extract_closures_from_expr(e, generated);
            }
        }
        StmtKind::Block(stmts) => {
            extract_closures_from_stmts(stmts, generated);
        }
        _ => {}
    }
}

/// Extract closures from an expression.
fn extract_closures_from_expr(expr: &mut Expr, generated: &mut Vec<FunctionDecl>) {
    match &mut expr.kind {
        ExprKind::MethodCall {
            object,
            method,
            args,
        } => {
            // Check if this is a route registration with a closure
            if is_http_method(method) && args.len() == 2 {
                if let ExprKind::Closure {
                    params,
                    body,
                    return_type,
                    error_type,
                } = &args[1].kind
                {
                    // Generate unique function name
                    let counter = CLOSURE_COUNTER.fetch_add(1, Ordering::SeqCst);
                    let path_hint = if let ExprKind::StrLit(path) = &args[0].kind {
                        path.replace('/', "_").replace('-', "_")
                    } else {
                        "handler".to_string()
                    };
                    let func_name = format!(
                        "__closure_{}_{}",
                        path_hint.trim_start_matches('_'),
                        counter
                    );

                    // Create function declaration
                    let body_stmts = if let ExprKind::Block(stmts, result) = &body.kind {
                        let mut stmts = stmts.clone();
                        if let Some(r) = result {
                            stmts.push(Stmt::new(
                                StmtKind::Return(vec![r.as_ref().clone()]),
                                body.span,
                            ));
                        }
                        stmts
                    } else {
                        if error_type.is_some() {
                            vec![Stmt::new(StmtKind::Expr(body.as_ref().clone()), body.span)]
                        } else {
                            vec![Stmt::new(
                                StmtKind::Return(vec![body.as_ref().clone()]),
                                body.span,
                            )]
                        }
                    };

                    let func_decl = FunctionDecl {
                        name: func_name.clone(),
                        is_public: false,
                        type_params: vec![],
                        params: params
                            .iter()
                            .map(|(name, ty)| (name.clone(), ty.clone()))
                            .collect(),
                        return_type: return_type.clone(),
                        error_type: error_type.clone(),
                        body: body_stmts,
                        decorators: vec![],
                        receiver: None,
                        associated_type: None,
                        is_expr_fn: false,
                        is_async: false,
                        span: body.span,
                    };

                    generated.push(func_decl);

                    // Replace closure with function name reference
                    args[1] = Expr::new(ExprKind::Ident(func_name), args[1].span);
                }
            }

            // Recurse
            extract_closures_from_expr(object, generated);
            for arg in args {
                extract_closures_from_expr(arg, generated);
            }
        }
        ExprKind::Call { func, args } => {
            extract_closures_from_expr(func, generated);
            for arg in args {
                extract_closures_from_expr(arg, generated);
            }
        }
        ExprKind::Binary { left, right, .. } => {
            extract_closures_from_expr(left, generated);
            extract_closures_from_expr(right, generated);
        }
        ExprKind::Unary { expr: e, .. } => {
            extract_closures_from_expr(e, generated);
        }
        ExprKind::Field { object, .. } => {
            extract_closures_from_expr(object, generated);
        }
        ExprKind::Index { object, index } => {
            extract_closures_from_expr(object, generated);
            extract_closures_from_expr(index, generated);
        }
        ExprKind::IfExpr {
            condition,
            then_branch,
            else_branch,
        } => {
            extract_closures_from_expr(condition, generated);
            extract_closures_from_expr(then_branch, generated);
            if let Some(e) = else_branch {
                extract_closures_from_expr(e, generated);
            }
        }
        ExprKind::Block(stmts, result) => {
            extract_closures_from_stmts(stmts, generated);
            if let Some(r) = result {
                extract_closures_from_expr(r, generated);
            }
        }
        ExprKind::ArrayLit(elems) | ExprKind::TupleLit(elems) => {
            for e in elems {
                extract_closures_from_expr(e, generated);
            }
        }
        ExprKind::MapLit(pairs) => {
            for (k, v) in pairs {
                extract_closures_from_expr(k, generated);
                extract_closures_from_expr(v, generated);
            }
        }
        ExprKind::ObjectLit(pairs) | ExprKind::StructLit { fields: pairs, .. } => {
            for (_, v) in pairs {
                extract_closures_from_expr(v, generated);
            }
        }
        _ => {}
    }
}

/// Recursively transform route groups in a statement.
fn transform_stmt_recursive(stmt: &mut Stmt) {
    match &mut stmt.kind {
        StmtKind::Expr(expr) => {
            transform_expr_recursive(expr);
        }
        StmtKind::Let { value, .. } => {
            transform_expr_recursive(value);
        }
        StmtKind::If {
            condition,
            then_block,
            else_branch,
        } => {
            transform_expr_recursive(condition);
            transform_stmts(then_block);
            if let Some(else_b) = else_branch {
                match else_b {
                    doo_frontend::ast::ElseBranch::Block(stmts) => {
                        transform_stmts(stmts);
                    }
                    doo_frontend::ast::ElseBranch::ElseIf(s) => {
                        transform_stmt_recursive(s);
                    }
                }
            }
        }
        StmtKind::For { body, iterable, .. } => {
            if let Some(iter) = iterable {
                transform_expr_recursive(iter);
            }
            transform_stmts(body);
        }
        StmtKind::Return(exprs) | StmtKind::Print(exprs) => {
            for e in exprs {
                transform_expr_recursive(e);
            }
        }
        StmtKind::Block(stmts) => {
            transform_stmts(stmts);
        }
        _ => {}
    }
}

/// Recursively transform route methods in an expression.
fn transform_expr_recursive(expr: &mut Expr) {
    match &mut expr.kind {
        ExprKind::MethodCall {
            object,
            method,
            args,
        } => {
            // Handle route registration methods
            if is_route_registration_method(method) {
                // Convert path params
                if !args.is_empty() {
                    if let ExprKind::StrLit(path) = &args[0].kind {
                        args[0] =
                            Expr::new(ExprKind::StrLit(convert_path_params(path)), args[0].span);
                    }
                }

                // Convert handlers to strings
                if args.len() == 2 {
                    args[1] = convert_handler_to_string(args[1].clone(), args[1].span);
                } else if args.len() > 2 && method != "use" {
                    // Route with middleware: transform to WithMiddleware variant
                    let middleware_count = args.len() - 2;
                    let mut middleware_names = Vec::new();
                    for i in 1..=middleware_count {
                        middleware_names.push(extract_identifier_name(&args[i]));
                    }
                    let middleware_str = middleware_names.join(",");
                    // Keep handler as-is (identifier) for function pointer passing
                    let handler = args[args.len() - 1].clone();

                    *method = format!("{}WithMiddleware", method);
                    *args = vec![
                        args[0].clone(),
                        Expr::new(ExprKind::StrLit(middleware_str), expr.span),
                        handler, // Keep as identifier, not string
                    ];
                } else if method == "use" && args.len() > 1 {
                    // Middleware chaining: app.use(M1, M2, M3) -> nested calls
                    let mut current = object.as_ref().clone();
                    for middleware in args.iter() {
                        let converted = convert_handler_to_string(middleware.clone(), expr.span);
                        current = Expr::new(
                            ExprKind::MethodCall {
                                object: Box::new(current),
                                method: "use".to_string(),
                                args: vec![converted],
                            },
                            expr.span,
                        );
                    }
                    *expr = current;
                    return;
                } else if method == "use" && args.len() == 1 {
                    args[0] = convert_handler_to_string(args[0].clone(), args[0].span);
                }

                // Handle auth() and crud() special methods
                if method == "auth" && args.is_empty() {
                    // app.auth() with no args — inject sensible defaults from centralized constants.
                    // Uses active DB pool if connected, in-memory auth otherwise.
                    args.push(Expr::new(ExprKind::StrLit(DEFAULT_AUTH_SIGNUP_PATH.into()), expr.span));
                    args.push(Expr::new(ExprKind::StrLit(DEFAULT_AUTH_LOGIN_PATH.into()), expr.span));
                    args.push(Expr::new(ExprKind::StrLit("".into()), expr.span));
                    args.push(Expr::new(ExprKind::StrLit("".into()), expr.span));
                } else if method == "auth" && args.len() == 4 {
                    // app.auth(signupPath, loginPath, UserStruct, db)
                    args[2] = convert_handler_to_string(args[2].clone(), args[2].span);
                } else if method == "crud" && args.len() == 3 {
                    // app.crud(basePath, ResourceStruct, db)
                    args[1] = convert_handler_to_string(args[1].clone(), args[1].span);
                }
            }

            // Recurse
            transform_expr_recursive(object);
            for arg in args {
                transform_expr_recursive(arg);
            }
        }
        ExprKind::Call { func, args } => {
            transform_expr_recursive(func);
            for arg in args {
                transform_expr_recursive(arg);
            }
        }
        ExprKind::Binary { left, right, .. } => {
            transform_expr_recursive(left);
            transform_expr_recursive(right);
        }
        ExprKind::Unary { expr: e, .. } => {
            transform_expr_recursive(e);
        }
        ExprKind::Field { object, .. } => {
            transform_expr_recursive(object);
        }
        ExprKind::Index { object, index } => {
            transform_expr_recursive(object);
            transform_expr_recursive(index);
        }
        ExprKind::IfExpr {
            condition,
            then_branch,
            else_branch,
        } => {
            transform_expr_recursive(condition);
            transform_expr_recursive(then_branch);
            if let Some(e) = else_branch {
                transform_expr_recursive(e);
            }
        }
        ExprKind::Block(stmts, result) => {
            transform_stmts(stmts);
            if let Some(r) = result {
                transform_expr_recursive(r);
            }
        }
        ExprKind::ArrayLit(elems) | ExprKind::TupleLit(elems) => {
            for e in elems {
                transform_expr_recursive(e);
            }
        }
        ExprKind::MapLit(pairs) => {
            for (k, v) in pairs {
                transform_expr_recursive(k);
                transform_expr_recursive(v);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_path_params() {
        assert_eq!(convert_path_params("/users/:id"), "/users/{id}");
        assert_eq!(
            convert_path_params("/users/:userId/posts/:postId"),
            "/users/{userId}/posts/{postId}"
        );
        assert_eq!(
            convert_path_params("/api/items/:id/details"),
            "/api/items/{id}/details"
        );
        assert_eq!(convert_path_params("/no/params"), "/no/params");
    }

    #[test]
    fn test_is_http_method() {
        assert!(is_http_method("get"));
        assert!(is_http_method("post"));
        assert!(is_http_method("put"));
        assert!(is_http_method("delete"));
        assert!(is_http_method("patch"));
        assert!(!is_http_method("foo"));
    }
}
