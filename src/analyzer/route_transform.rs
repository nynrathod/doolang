//! Route Group DSL Transformation
//!
//! This module transforms the route group DSL syntax into individual route registrations.
//!
//! Example transformation:
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

use crate::parser::ast::AstNode;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Global counter for generating unique closure function names
static CLOSURE_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Transform route group DSL syntax in the AST
///
/// This function walks the AST and expands any `app.group(prefix, block)` calls
/// into individual `app.get()`, `app.post()`, etc. calls with the prefix prepended.
pub fn transform_route_groups(nodes: &mut Vec<AstNode>) {
    let mut i = 0;
    while i < nodes.len() {
        if let Some(expanded) = try_expand_route_group(&nodes[i]) {
            // Replace the single group() call with multiple route calls
            let expanded_len = expanded.len();
            nodes.splice(i..i + 1, expanded);
            i += expanded_len;
        } else {
            // Recursively transform children
            transform_route_group_in_node(&mut nodes[i]);
            i += 1;
        }
    }
}

/// Try to expand a route group method call
///
/// Returns Some(vec![expanded_calls]) if this is a group() call that can be expanded,
/// None otherwise.
fn try_expand_route_group(node: &AstNode) -> Option<Vec<AstNode>> {
    // Check if this is a method call to "group"
    if let AstNode::MethodCall {
        object,
        method,
        args,
    } = node
    {
        // Must be calling "group" method
        if method != "group" {
            return None;
        }

        // Must have exactly 2 arguments: prefix and block
        if args.len() != 2 {
            return None;
        }

        // Extract the prefix (first argument)
        let prefix_expr = &args[0];

        // Extract the block (second argument)
        let block = &args[1];

        // Extract route definitions from the block
        let route_calls = match block {
            AstNode::BlockExpr { statements, result } => {
                let mut calls = extract_route_calls_from_statements(statements);
                // Also check the result expression if it's not nil
                if !matches!(result.as_ref(), AstNode::NilLiteral) {
                    if let Some(call) = extract_route_call(result) {
                        calls.push(call);
                    }
                }
                calls
            }
            // Single expression block (shouldn't happen but handle it)
            _ => {
                if let Some(call) = extract_route_call(block) {
                    vec![call]
                } else {
                    vec![]
                }
            }
        };

        // Transform each route call into a method call on the original object
        let expanded: Vec<AstNode> = route_calls
            .into_iter()
            .map(|(http_method, path_expr, handler_expr)| {
                // Create the full path: prefix + path
                let full_path = create_path_concat(prefix_expr, &path_expr);

                // Convert handler identifier to string literal
                let handler_str = convert_handler_to_string(handler_expr);

                // Create the method call: object.method(full_path, handler)
                AstNode::MethodCall {
                    object: Box::new(object.as_ref().clone()),
                    method: http_method,
                    args: vec![full_path, handler_str],
                }
            })
            .collect();

        if !expanded.is_empty() {
            return Some(expanded);
        }
    }

    None
}

/// Extract route calls from a list of statements
fn extract_route_calls_from_statements(statements: &[AstNode]) -> Vec<(String, AstNode, AstNode)> {
    statements
        .iter()
        .filter_map(|stmt| extract_route_call(stmt))
        .collect()
}

/// Extract a route call (method, path, handler) from an AST node
///
/// Looks for function calls like: get("/path", handler) or post("/path", handler)
/// Returns (http_method, path_expr, handler_expr)
fn extract_route_call(node: &AstNode) -> Option<(String, AstNode, AstNode)> {
    match node {
        AstNode::FunctionCall { func, args } => {
            // Extract function name
            let method_name = match func.as_ref() {
                AstNode::Identifier(name) => name,
                _ => return None,
            };

            // Must be a known HTTP method
            if !is_http_method(method_name) {
                return None;
            }

            // Must have exactly 2 arguments: path and handler
            if args.len() != 2 {
                return None;
            }

            let path_expr = args[0].clone();
            let handler_expr = args[1].clone();

            Some((method_name.clone(), path_expr, handler_expr))
        }
        _ => None,
    }
}

/// Check if a string is a valid HTTP method name
fn is_http_method(name: &str) -> bool {
    matches!(name, "get" | "post" | "put" | "delete" | "patch")
}

/// Check if a method name is a route registration method
fn is_route_registration_method(name: &str) -> bool {
    matches!(name, "get" | "post" | "put" | "delete" | "patch" | "group")
}

/// Create a path concatenation expression: prefix + path
///
/// If prefix is a string literal "/api" and path is "/users",
/// this creates a string concatenation that results in "/api/users"
fn create_path_concat(prefix: &AstNode, path: &AstNode) -> AstNode {
    // If both are string literals, we can optimize by concatenating at compile time
    if let (AstNode::StringLiteral(prefix_str), AstNode::StringLiteral(path_str)) = (prefix, path) {
        // Ensure proper path joining (no double slashes)
        let full_path = if path_str.starts_with('/') {
            format!("{}{}", prefix_str, path_str)
        } else {
            format!("{}/{}", prefix_str, path_str)
        };
        return AstNode::StringLiteral(full_path);
    }

    // Otherwise, create a runtime concatenation expression
    AstNode::BinaryExpr {
        left: Box::new(prefix.clone()),
        op: crate::lexer::token::TokenType::Plus,
        right: Box::new(path.clone()),
    }
}

/// Convert a handler expression to a string literal
///
/// If the handler is an identifier (function name), convert it to a string literal.
/// For closures, they will be transformed to functions first by transform_inline_closures.
fn convert_handler_to_string(handler: AstNode) -> AstNode {
    match handler {
        AstNode::Identifier(name) => AstNode::StringLiteral(name),
        // Closures should have been transformed already, but handle them gracefully
        AstNode::Closure { .. } => {
            eprintln!("Warning: Untransformed closure found in handler position");
            handler
        }
        other => other,
    }
}

/// Transform inline closures in route registrations into named function declarations
///
/// Finds patterns like:
///   app.post("/path", (params) -> ReturnType { body })
///
/// And transforms to:
///   fn __closure_path_N(params) -> ReturnType { body }
///   app.post("/path", "__closure_path_N")
pub fn transform_inline_closures(nodes: &mut Vec<AstNode>) {
    let mut new_functions = Vec::new();

    // Process all nodes and collect generated functions
    for node in nodes.iter_mut() {
        extract_and_transform_closures(node, &mut new_functions);
    }

    // Insert generated functions at the beginning
    for func in new_functions.into_iter().rev() {
        nodes.insert(0, func);
    }
}

/// Recursively find and transform closures in route registrations
fn extract_and_transform_closures(node: &mut AstNode, generated_functions: &mut Vec<AstNode>) {
    match node {
        AstNode::MethodCall {
            object,
            method,
            args,
        } => {
            // Check if this is a route registration method (get, post, put, delete, patch)
            if is_http_method(method) && args.len() == 2 {
                // Check if second argument is a closure
                if let AstNode::Closure {
                    params,
                    body,
                    return_type,
                    error_type,
                } = &args[1]
                {
                    // Generate unique function name
                    let counter = CLOSURE_COUNTER.fetch_add(1, Ordering::SeqCst);
                    let path_hint = if let AstNode::StringLiteral(path) = &args[0] {
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
                    // Convert closure body (Box<AstNode>) to function body (Vec<AstNode>)
                    let body_vec = if let AstNode::Block(stmts) = body.as_ref() {
                        // Block body - keep as-is
                        // User will write `return X` for plain types or `Ok X; Err Y;` for Result types
                        stmts.clone()
                    } else {
                        // Single expression body
                        // Functions with error types use Ok/Err expressions (already present)
                        // Functions without error types need Return statement
                        if error_type.is_some() {
                            // Has error type - Ok/Err expressions are valid as-is
                            vec![*body.clone()]
                        } else {
                            // No error type - wrap in Return statement
                            vec![AstNode::Return {
                                values: vec![*body.clone()],
                            }]
                        }
                    };

                    let func_decl = AstNode::FunctionDecl {
                        name: func_name.clone(),
                        visibility: String::new(),
                        params: params.clone(),
                        return_type: return_type.clone(),
                        error_type: error_type.clone(),
                        body: body_vec,
                        decorators: vec![],
                        receiver_type: None,
                        is_expression: false,
                    };

                    generated_functions.push(func_decl);

                    // Replace closure with function name identifier
                    args[1] = AstNode::Identifier(func_name);
                }
            }

            // Recursively process object and args
            extract_and_transform_closures(object, generated_functions);
            for arg in args.iter_mut() {
                extract_and_transform_closures(arg, generated_functions);
            }
        }
        AstNode::FunctionDecl { body, .. } => {
            for stmt in body.iter_mut() {
                extract_and_transform_closures(stmt, generated_functions);
            }
        }
        AstNode::Block(statements) => {
            for stmt in statements.iter_mut() {
                extract_and_transform_closures(stmt, generated_functions);
            }
        }
        AstNode::LetDecl { value, .. } => {
            extract_and_transform_closures(value, generated_functions);
        }
        AstNode::Assignment { value, .. } => {
            extract_and_transform_closures(value, generated_functions);
        }
        AstNode::ConditionalStmt {
            condition,
            then_block,
            else_branch,
        } => {
            extract_and_transform_closures(condition, generated_functions);
            for stmt in then_block.iter_mut() {
                extract_and_transform_closures(stmt, generated_functions);
            }
            if let Some(else_blk) = else_branch {
                extract_and_transform_closures(else_blk, generated_functions);
            }
        }
        AstNode::ForLoopStmt { body, .. } => {
            for stmt in body.iter_mut() {
                extract_and_transform_closures(stmt, generated_functions);
            }
        }
        AstNode::Return { values } => {
            for val in values.iter_mut() {
                extract_and_transform_closures(val, generated_functions);
            }
        }
        AstNode::FunctionCall { args, .. } => {
            for arg in args.iter_mut() {
                extract_and_transform_closures(arg, generated_functions);
            }
        }
        AstNode::BinaryExpr { left, right, .. } => {
            extract_and_transform_closures(left, generated_functions);
            extract_and_transform_closures(right, generated_functions);
        }
        AstNode::UnaryExpr { expr, .. } => {
            extract_and_transform_closures(expr, generated_functions);
        }
        AstNode::FieldAccess { object, .. } => {
            extract_and_transform_closures(object, generated_functions);
        }
        AstNode::ElementAccess { array, index } => {
            extract_and_transform_closures(array, generated_functions);
            extract_and_transform_closures(index, generated_functions);
        }
        AstNode::TryPropagate { expr } => {
            extract_and_transform_closures(expr, generated_functions);
        }
        AstNode::OkExpr { values } => {
            for val in values.iter_mut() {
                extract_and_transform_closures(val, generated_functions);
            }
        }
        AstNode::ErrExpr { value } => {
            extract_and_transform_closures(value, generated_functions);
        }
        _ => {}
    }
}

/// Recursively transform route groups in child nodes
fn transform_route_group_in_node(node: &mut AstNode) {
    match node {
        AstNode::FunctionDecl { body, .. } => {
            transform_route_groups(body);
        }
        AstNode::Block(statements) => {
            transform_route_groups(statements);
        }
        AstNode::BlockExpr { statements, result } => {
            transform_route_groups(statements);
            transform_route_group_in_node(result);
        }
        AstNode::ConditionalStmt {
            then_block,
            else_branch,
            ..
        } => {
            transform_route_groups(then_block);
            if let Some(else_node) = else_branch {
                transform_route_group_in_node(else_node);
            }
        }
        AstNode::ConditionalExpr {
            then_expr,
            else_expr,
            ..
        } => {
            transform_route_group_in_node(then_expr);
            transform_route_group_in_node(else_expr);
        }
        AstNode::ForLoopStmt { body, .. } => {
            transform_route_groups(body);
        }
        AstNode::MatchExpr { arms, .. } => {
            for arm in arms {
                // MatchArm is a separate struct with a body field
                match arm {
                    crate::parser::ast::MatchArm { body, .. } => {
                        transform_route_group_in_node(body);
                    }
                }
            }
        }
        AstNode::MethodCall {
            object,
            method,
            args,
        } => {
            // For route registration methods, convert handler identifiers to strings
            if is_route_registration_method(method) && args.len() == 2 {
                transform_route_group_in_node(object);
                transform_route_group_in_node(&mut args[0]); // path
                                                             // Convert handler (second arg) from identifier to string
                args[1] = convert_handler_to_string(args[1].clone());
            } else {
                transform_route_group_in_node(object);
                for arg in args {
                    transform_route_group_in_node(arg);
                }
            }
        }
        AstNode::FunctionCall { func, args } => {
            transform_route_group_in_node(func);
            for arg in args {
                transform_route_group_in_node(arg);
            }
        }
        AstNode::Assignment { value, .. } => {
            transform_route_group_in_node(value);
        }
        AstNode::CompoundAssignment { value, .. } => {
            transform_route_group_in_node(value);
        }
        AstNode::Return { values } => {
            for val in values {
                transform_route_group_in_node(val);
            }
        }
        AstNode::BinaryExpr { left, right, .. } => {
            transform_route_group_in_node(left);
            transform_route_group_in_node(right);
        }
        AstNode::UnaryExpr { expr, .. } => {
            transform_route_group_in_node(expr);
        }
        AstNode::ArrayLiteral(elements) => {
            for elem in elements {
                transform_route_group_in_node(elem);
            }
        }
        AstNode::MapLiteral(pairs) => {
            for (key, value) in pairs {
                transform_route_group_in_node(key);
                transform_route_group_in_node(value);
            }
        }
        AstNode::StructLiteral { fields, .. } => {
            for (_, value) in fields {
                transform_route_group_in_node(value);
            }
        }
        AstNode::FieldAccess { object, .. } => {
            transform_route_group_in_node(object);
        }
        AstNode::ElementAccess { array, index } => {
            transform_route_group_in_node(array);
            transform_route_group_in_node(index);
        }
        AstNode::Closure { body, .. } => {
            transform_route_group_in_node(body);
        }
        AstNode::TernaryExpr {
            condition,
            true_expr,
            false_expr,
        } => {
            transform_route_group_in_node(condition);
            transform_route_group_in_node(true_expr);
            transform_route_group_in_node(false_expr);
        }
        AstNode::TryPropagate { expr } => {
            transform_route_group_in_node(expr);
        }
        AstNode::UnwrapOrPanic { expr, panic_msg } => {
            transform_route_group_in_node(expr);
            transform_route_group_in_node(panic_msg);
        }
        AstNode::Cast { expr, .. } => {
            transform_route_group_in_node(expr);
        }
        AstNode::SpreadElement(expr) => {
            transform_route_group_in_node(expr);
        }
        AstNode::Range { start, end, .. } => {
            transform_route_group_in_node(start);
            transform_route_group_in_node(end);
        }
        AstNode::EnumVariant { payload, .. } => {
            for arg in payload {
                transform_route_group_in_node(arg);
            }
        }
        AstNode::FieldAssignment { object, value, .. } => {
            transform_route_group_in_node(object);
            transform_route_group_in_node(value);
        }
        AstNode::ElementAssignment {
            array,
            index,
            value,
        } => {
            transform_route_group_in_node(array);
            transform_route_group_in_node(index);
            transform_route_group_in_node(value);
        }
        AstNode::Print { exprs } => {
            for expr in exprs {
                transform_route_group_in_node(expr);
            }
        }
        // Leaf nodes - no transformation needed
        AstNode::Identifier(_)
        | AstNode::NumberLiteral(_)
        | AstNode::FloatLiteral(_)
        | AstNode::StringLiteral(_)
        | AstNode::BoolLiteral(_)
        | AstNode::NilLiteral
        | AstNode::Break
        | AstNode::Continue
        | AstNode::StructDecl { .. }
        | AstNode::EnumDecl { .. }
        | AstNode::Import { .. }
        | AstNode::IncrementDecrement { .. }
        | AstNode::ErrExpr { .. }
        | AstNode::Program(_)
        | AstNode::LetDecl { .. }
        | AstNode::TupleLiteral(_)
        | AstNode::OkExpr { .. }
        | AstNode::ManualErrorExtract { .. } => {}
    }
}
