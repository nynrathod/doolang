//! HTTP route middleware helpers.

use doo_core::{
    doo_debug,
    infer::{infer_binop_result_type, infer_unaryop_result_type, BinOpKind, UnaryOpKind},
    types::{builtin, TypeId, TypeKind, TypeRegistry},
    Span,
};
use doo_frontend::ast::{
    self, BinaryOp, CompoundOp, Decorator, ElseBranch, EnumDecl, Expr, ExprKind, FunctionDecl,
    ImportDecl, IncDecOp, Item, Pattern, PatternKind, Program, Stmt, StmtKind, StructDecl,
    TypeExpr, UnaryOp,
};
use crate::types::*;
use super::{Lower, LowerError};

impl Lower {
    /// Check if a method name is an HTTP route method.
    pub(crate) fn is_http_route_method(method: &str) -> bool {
        matches!(
            method,
            "get" | "post" | "put" | "delete" | "patch" | "head" | "options"
        )
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

        // Collect middleware names (args[1..len-1]) as comma-separated string
        // Each middleware arg should be a call that returns a string (e.g., jwt() returns "jwt")
        let middleware_args: Vec<HirExpr> = args[1..args.len() - 1]
            .iter()
            .map(|a| self.lower_expr(a))
            .collect();

        // For single middleware, pass it directly
        // For multiple middlewares, we'd need to concatenate strings
        // For now, use the first middleware if any
        let middleware = if middleware_args.is_empty() {
            HirExpr::new(HirExprKind::Const(ConstValue::Str("".to_string())), span)
        } else if middleware_args.len() == 1 {
            middleware_args.into_iter().next().unwrap()
        } else {
            // TODO: Concatenate multiple middleware names
            middleware_args.into_iter().next().unwrap()
        };

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
    pub(crate) fn transform_group_with_routes(&mut self, object: &Expr, args: &[Expr], span: Span) -> HirExpr {
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
        // args format: [path, middleware1, middleware2, ..., handler]
        // Transform to: Server.{method}WithMiddleware(path, middleware_str, handler)
        let receiver = Box::new(self.lower_expr_typed(object, registry));
        let path = self.lower_expr_typed(&args[0], registry);
        let handler = self.lower_expr_typed(args.last().unwrap(), registry);

        // Collect middleware names (args[1..len-1])
        let middleware_args: Vec<HirExpr> = args[1..args.len() - 1]
            .iter()
            .map(|a| self.lower_expr_typed(a, registry))
            .collect();

        // For single middleware, pass it directly
        let middleware = if middleware_args.is_empty() {
            HirExpr::new(HirExprKind::Const(ConstValue::Str("".to_string())), span)
        } else if middleware_args.len() == 1 {
            middleware_args.into_iter().next().unwrap()
        } else {
            // TODO: Concatenate multiple middleware names
            middleware_args.into_iter().next().unwrap()
        };

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
}
