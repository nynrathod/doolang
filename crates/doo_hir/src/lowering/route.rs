//! HTTP route middleware helpers.

use super::Lower;
use crate::types::*;
use doo_core::{
    types::{builtin, TypeRegistry},
    Span,
};
use doo_frontend::ast::{Expr, ExprKind};

impl Lower {
    /// Check if a method name is an HTTP route method.
    pub(crate) fn is_http_route_method(method: &str) -> bool {
        matches!(
            method,
            "get" | "post" | "put" | "delete" | "patch" | "head" | "options"
        )
    }

    /// Extract the middleware name from an AST expression.
    /// For identifiers like `AuthMiddleware`, returns the name as-is.
    /// For calls like `Jwt()`, returns "Jwt".
    /// This is needed because the FFI expects middleware names as strings,
    /// not as function references.
    fn extract_middleware_name(expr: &Expr) -> String {
        match &expr.kind {
            ExprKind::Ident(name) => name.clone(),
            ExprKind::Call { func, .. } => {
                if let ExprKind::Ident(name) = &func.kind {
                    name.clone()
                } else {
                    "unknown".to_string()
                }
            }
            _ => "unknown".to_string(),
        }
    }

    /// Build a middleware string HIR expression from AST middleware arguments.
    /// Extracts names from identifiers/calls and joins with commas.
    fn build_middleware_str(&self, middleware_asts: &[Expr], span: Span) -> HirExpr {
        if middleware_asts.is_empty() {
            return HirExpr::new(HirExprKind::Const(ConstValue::Str("".to_string())), span);
        }
        let names: Vec<String> = middleware_asts
            .iter()
            .map(|a| Self::extract_middleware_name(a))
            .collect();
        HirExpr::new(HirExprKind::Const(ConstValue::Str(names.join(","))), span)
    }

    /// Transform a route method call with middleware arguments.
    /// app.get("/path", middleware, Handler) -> HirExpr for method call with middleware array
    pub(crate) fn transform_route_with_middleware(
        &mut self,
        object: &Expr,
        method: &str,
        args: &[Expr],
        span: Span,
    ) -> HirExpr {
        // args format: [path, middleware1, middleware2, ..., handler]
        // Transform to: Server.{method}WithMiddleware(path, middleware_str, handler)
        let receiver = Box::new(self.lower_expr(object));
        let path = self.lower_expr(&args[0]);
        let handler = self.lower_expr(args.last().unwrap());

        // Extract middleware names (args[1..len-1]) as comma-separated string
        let middleware_asts = &args[1..args.len() - 1];
        let middleware = self.build_middleware_str(middleware_asts, span);

        // Create method name with "WithMiddleware" suffix
        let new_method = format!("{}WithMiddleware", method);

        // Args: [path, middleware, handler]
        let new_args = vec![path, middleware, handler];

        HirExpr::new(
            HirExprKind::MethodCall {
                receiver,
                method: new_method,
                args: new_args,
            },
            span,
        )
    }

    /// Transform app.group with a route block.
    /// app.group("/api", middleware, { get(...), post(...) }) -> expand each route
    pub(crate) fn transform_group_with_routes(
        &mut self,
        object: &Expr,
        args: &[Expr],
        span: Span,
    ) -> HirExpr {
        // For now, just lower the group call normally, the FFI will handle it at runtime
        // args[0] = prefix, args[1..n-1] = middleware, args[n-1] = RouteBlock
        let receiver = Box::new(self.lower_expr(object));
        let lowered_args: Vec<HirExpr> = args.iter().map(|a| self.lower_expr(a)).collect();

        HirExpr::new(
            HirExprKind::MethodCall {
                receiver,
                method: "group".to_string(),
                args: lowered_args,
            },
            span,
        )
    }

    /// Typed version: Transform route with middleware
    pub(crate) fn transform_route_with_middleware_typed(
        &mut self,
        object: &Expr,
        method: &str,
        args: &[Expr],
        span: Span,
        registry: &mut TypeRegistry,
    ) -> HirExpr {
        // ── Webhook detection (belt + suspenders) ────────────────────────
        // Same heuristic as lower_expr_typed: second-to-last is handler (known fn),
        // last is webhook JSON if it's NOT a known function name.
        if args.len() >= 3 {
            let handler_idx = args.len() - 2;
            let webhook_idx = args.len() - 1;

            let handler_is_known_fn = match &args[handler_idx].kind {
                ExprKind::Ident(name) => {
                    let c = self.known_functions.contains(name.as_str());
                    c
                }
                other => false,
            };

            let last_is_not_fn = match &args[webhook_idx].kind {
                ExprKind::StrLit(_) => true,
                ExprKind::Ident(name) => {
                    let n = !self.known_functions.contains(name.as_str());
                    n
                }
                ExprKind::Call { .. } | ExprKind::MethodCall { .. } => true,
                _ => {
                    self.expr_is_str(&args[webhook_idx]) || {
                        let lowered_last = self.lower_expr_typed(&args[webhook_idx], registry);
                        lowered_last.type_id.map_or(false, |tid| {
                            tid == builtin::STR
                                || registry.get(tid).map_or(false, |info| info.name == "Str")
                        })
                    }
                }
            };

            if handler_is_known_fn && last_is_not_fn {
                return self
                    .transform_route_with_webhook_typed(object, method, args, span, registry);
            }
        }

        // args format: [path, middleware1, middleware2, ..., handler]
        // Transform to: Server.{method}WithMiddleware(path, middleware_str, handler)
        let receiver = Box::new(self.lower_expr_typed(object, registry));
        let path = self.lower_expr_typed(&args[0], registry);
        let handler = self.lower_expr_typed(args.last().unwrap(), registry);

        // Extract middleware names (args[1..len-1]) as comma-separated string
        let middleware_asts = &args[1..args.len() - 1];
        let middleware = self.build_middleware_str(middleware_asts, span);

        // Create method name with "WithMiddleware" suffix
        let new_method = format!("{}WithMiddleware", method);

        // Args: [path, middleware, handler]
        let new_args = vec![path, middleware, handler];

        HirExpr::new(
            HirExprKind::MethodCall {
                receiver,
                method: new_method,
                args: new_args,
            },
            span,
        )
    }

    /// Typed version: Transform app.group with a route block
    pub(crate) fn transform_group_with_routes_typed(
        &mut self,
        object: &Expr,
        args: &[Expr],
        span: Span,
        registry: &mut TypeRegistry,
    ) -> HirExpr {
        // For now, just lower the group call normally
        let receiver = Box::new(self.lower_expr_typed(object, registry));
        let lowered_args: Vec<HirExpr> = args
            .iter()
            .map(|a| self.lower_expr_typed(a, registry))
            .collect();

        HirExpr::new(
            HirExprKind::MethodCall {
                receiver,
                method: "group".to_string(),
                args: lowered_args,
            },
            span,
        )
    }

    /// Typed version: Transform a route method call that includes a webhook JSON
    /// string as the LAST argument.
    ///
    /// ## Pattern
    /// - `app.get(path, handler, webhooksJson)`  — no middleware, with webhook
    /// - `app.get(path, Jwt(), handler, webhooksJson)` — with middleware, with webhook
    /// - `app.get(path, mw1, mw2, handler, webhooksJson)` — multi middleware, with webhook
    ///
    /// ## Transformation
    /// Produces a HIR Block with TWO method calls:
    /// 1. The route registration call (get / getWithMiddleware / etc.)
    /// 2. The webhook registration call (webhookForRoute)
    ///
    /// This preserves the existing FFI surface — no new FFI symbols needed.
    pub(crate) fn transform_route_with_webhook_typed(
        &mut self,
        object: &Expr,
        method: &str,
        args: &[Expr],
        span: Span,
        registry: &mut TypeRegistry,
    ) -> HirExpr {
        // args layout: [path, ...middlewares..., handler, webhooksJson]
        let n = args.len();
        let handler_idx = n - 2; // second-to-last is the handler
        let webhook_idx = n - 1; // last is the webhook JSON

        // Lower the server object once, clone for reuse
        let receiver = self.lower_expr_typed(object, registry);

        // Detect middleware args (everything between path and handler)
        let has_middleware = handler_idx > 1;
        let middleware_asts: &[Expr] = if has_middleware {
            &args[1..handler_idx]
        } else {
            &[]
        };

        // ── Route registration call ──────────────────────────────────────
        let route_call = if has_middleware {
            // Route with middleware: app.getWithMiddleware(path, middlewareStr, handler)
            let path = self.lower_expr_typed(&args[0], registry);
            let handler = self.lower_expr_typed(&args[handler_idx], registry);
            let middleware = self.build_middleware_str(middleware_asts, span);
            HirExpr::new(
                HirExprKind::MethodCall {
                    receiver: Box::new(receiver.clone()),
                    method: format!("{}WithMiddleware", method),
                    args: vec![path, middleware, handler],
                },
                span,
            )
        } else {
            // Simple route: app.get(path, handler)
            let path = self.lower_expr_typed(&args[0], registry);
            let handler = self.lower_expr_typed(&args[handler_idx], registry);
            HirExpr::new(
                HirExprKind::MethodCall {
                    receiver: Box::new(receiver.clone()),
                    method: method.to_string(),
                    args: vec![path, handler],
                },
                span,
            )
        };

        // ── Webhook registration call ────────────────────────────────────
        let path = self.lower_expr_typed(&args[0], registry);
        let webhooks_json = self.lower_expr_typed(&args[webhook_idx], registry);
        let http_method = HirExpr::new(
            HirExprKind::Const(ConstValue::Str(method.to_uppercase())),
            span,
        );
        let webhook_call = HirExpr::new(
            HirExprKind::MethodCall {
                receiver: Box::new(receiver),
                method: "webhookForRoute".to_string(),
                args: vec![http_method, path, webhooks_json],
            },
            span,
        );

        // ── Wrap both calls in a Block ────────────────────────────────────
        HirExpr::new(
            HirExprKind::Block {
                stmts: vec![
                    HirStmt::new(HirStmtKind::Expr(route_call), span),
                    HirStmt::new(HirStmtKind::Expr(webhook_call), span),
                ],
                expr: None,
            },
            span,
        )
    }

    /// Check if an AST expression represents a `Str` value (used to detect
    /// webhook JSON arguments in route method calls).
    pub(crate) fn expr_is_str(&self, expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::StrLit(_) => true,
            ExprKind::Ident(name) => {
                // Check if this identifier has been tracked as a Str variable
                self.var_types.get(name) == Some(&doo_core::types::builtin::STR)
            }
            _ => false,
        }
    }
}
