//! For-loop desugaring to while-loops.

use super::Lower;
use crate::types::*;
use doo_core::{
    types::{builtin, TypeKind, TypeRegistry},
    Span,
};
use doo_frontend::ast::{Expr, ExprKind, Pattern, PatternKind, Stmt};

impl Lower {
    /// Lower a for-loop to HIR.
    ///
    /// ## Desugaring Rules
    ///
    /// ### Range iteration: `for i in start..end { body }`
    /// ```text
    /// let __i = start
    /// while __i < end {
    ///     let i = __i
    ///     body
    ///     __i = __i + 1
    /// }
    /// ```
    ///
    /// ### Array iteration: `for x in array { body }`
    /// ```text
    /// let __arr = array
    /// let __idx = 0
    /// while __idx < __arr.len() {
    ///     let x = __arr[__idx]
    ///     body
    ///     __idx = __idx + 1
    /// }
    /// ```
    ///
    /// ### Array iteration with index: `for i, x in array { body }`
    /// ```text
    /// let __arr = array
    /// let __idx = 0
    /// while __idx < __arr.len() {
    ///     let i = __idx
    ///     let x = __arr[__idx]
    ///     body
    ///     __idx = __idx + 1
    /// }
    /// ```
    pub(crate) fn lower_for_loop(
        &mut self,
        pattern: &Pattern,
        iterable: Option<&Expr>,
        body: &[Stmt],
        span: Span,
    ) -> HirStmtKind {
        // No iterable = infinite loop
        let Some(iter_expr) = iterable else {
            let body_stmts: Vec<_> = body.iter().map(|s| self.lower_stmt(s)).collect();
            return HirStmtKind::While {
                condition: HirExpr::new(HirExprKind::Const(ConstValue::Bool(true)), span),
                body: body_stmts,
                increment: vec![],
            };
        };

        // Check if iterating over a range expression
        if let ExprKind::Range {
            start,
            end,
            inclusive,
        } = &iter_expr.kind
        {
            return self.lower_range_for_loop(pattern, start, end, *inclusive, body, span);
        }

        // Check if iterating over a map literal
        if matches!(&iter_expr.kind, ExprKind::MapLit(_)) {
            return self.lower_map_for_loop(pattern, iter_expr, body, span);
        }

        // Array/collection iteration
        self.lower_array_for_loop(pattern, iter_expr, body, span)
    }
    /// Lower range-based for-loop: `for i in start..end`
    pub(crate) fn lower_range_for_loop(
        &mut self,
        pattern: &Pattern,
        start: &Expr,
        end: &Expr,
        inclusive: bool,
        body: &[Stmt],
        span: Span,
    ) -> HirStmtKind {
        let iter_var = self.pattern_to_name(pattern);
        let internal_idx = format!("__{}_idx", iter_var);

        // Lower body statements
        let mut body_stmts: Vec<_> = body.iter().map(|s| self.lower_stmt(s)).collect();

        // Prepend: let iter_var = __idx
        body_stmts.insert(
            0,
            HirStmt::new(
                HirStmtKind::Let {
                    name: iter_var.clone(),
                    type_id: None,
                    value: HirExpr::new(
                        HirExprKind::Local {
                            name: internal_idx.clone(),
                        },
                        span,
                    ),
                    mutable: false,
                    ownership: Ownership::Owned,
                },
                span,
            ),
        );

        // Append: __idx = __idx + 1
        let increment_stmt = HirStmt::new(
            HirStmtKind::Assign {
                target: HirExpr::new(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    span,
                ),
                value: HirExpr::new(
                    HirExprKind::BinOp {
                        op: HirBinOp::Add,
                        lhs: Box::new(HirExpr::new(
                            HirExprKind::Local {
                                name: internal_idx.clone(),
                            },
                            span,
                        )),
                        rhs: Box::new(HirExpr::new(HirExprKind::Const(ConstValue::Int(1)), span)),
                    },
                    span,
                ),
            },
            span,
        );

        // Condition: __idx < end (or __idx <= end for inclusive)
        let cmp_op = if inclusive {
            HirBinOp::LtEq
        } else {
            HirBinOp::Lt
        };
        let condition = HirExpr::new(
            HirExprKind::BinOp {
                op: cmp_op,
                lhs: Box::new(HirExpr::new(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    span,
                )),
                rhs: Box::new(self.lower_expr(end)),
            },
            span,
        );

        // Build desugared block:
        // {
        //     let __idx = start
        //     while __idx < end { let i = __idx; body; __idx++ }
        // }
        let init_stmt = HirStmt::new(
            HirStmtKind::Let {
                name: internal_idx,
                type_id: None,
                value: self.lower_expr(start),
                mutable: true,
                ownership: Ownership::Owned,
            },
            span,
        );

        let while_stmt = HirStmt::new(
            HirStmtKind::While {
                condition,
                body: body_stmts,
                increment: vec![increment_stmt],
            },
            span,
        );

        // Return as block expression containing init + while
        HirStmtKind::Expr(HirExpr::new(
            HirExprKind::Block {
                stmts: vec![init_stmt, while_stmt],
                expr: None,
            },
            span,
        ))
    }

    /// Lower array-based for-loop: `for x in array` or `for i, x in array`
    pub(crate) fn lower_array_for_loop(
        &mut self,
        pattern: &Pattern,
        array_expr: &Expr,
        body: &[Stmt],
        span: Span,
    ) -> HirStmtKind {
        // Check if pattern is tuple (i, x) or single (x)
        let (index_var, elem_var) = match &pattern.kind {
            PatternKind::Tuple(patterns) if patterns.len() == 2 => {
                let idx = self.pattern_to_name(&patterns[0]);
                let elem = self.pattern_to_name(&patterns[1]);
                (Some(idx), elem)
            }
            _ => (None, self.pattern_to_name(pattern)),
        };

        let internal_idx = format!("__{}_idx", elem_var);
        let internal_arr = format!("__{}_arr", elem_var);
        let internal_len = format!("__{}_len", elem_var);

        // Lower body statements
        let mut body_stmts: Vec<_> = body.iter().map(|s| self.lower_stmt(s)).collect();

        // Prepend index assignment if pattern has index: let i = __idx
        if let Some(idx_name) = &index_var {
            body_stmts.insert(
                0,
                HirStmt::new(
                    HirStmtKind::Let {
                        name: idx_name.clone(),
                        type_id: None,
                        value: HirExpr::new(
                            HirExprKind::Local {
                                name: internal_idx.clone(),
                            },
                            span,
                        ),
                        mutable: false,
                        ownership: Ownership::Owned,
                    },
                    span,
                ),
            );
        }

        // Prepend element extraction: let x = __arr[__idx]
        let elem_extraction = HirStmt::new(
            HirStmtKind::Let {
                name: elem_var.clone(),
                type_id: None,
                value: HirExpr::new(
                    HirExprKind::Index {
                        object: Box::new(HirExpr::new(
                            HirExprKind::Local {
                                name: internal_arr.clone(),
                            },
                            span,
                        )),
                        index: Box::new(HirExpr::new(
                            HirExprKind::Local {
                                name: internal_idx.clone(),
                            },
                            span,
                        )),
                    },
                    span,
                ),
                mutable: false,
                ownership: Ownership::Owned,
            },
            span,
        );
        // Insert after index assignment if present, else at start
        let insert_pos = if index_var.is_some() { 1 } else { 0 };
        body_stmts.insert(insert_pos, elem_extraction);

        // Append: __idx = __idx + 1
        // Append: __idx = __idx + 1
        let increment_stmt = HirStmt::new(
            HirStmtKind::Assign {
                target: HirExpr::new(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    span,
                ),
                value: HirExpr::new(
                    HirExprKind::BinOp {
                        op: HirBinOp::Add,
                        lhs: Box::new(HirExpr::new(
                            HirExprKind::Local {
                                name: internal_idx.clone(),
                            },
                            span,
                        )),
                        rhs: Box::new(HirExpr::new(HirExprKind::Const(ConstValue::Int(1)), span)),
                    },
                    span,
                ),
            },
            span,
        );

        // Condition: __idx < __len (pre-computed length, avoids array access
        // in the loop header which triggers auto-cloning and generates complex
        // LLVM IR that causes O3 to remove the loop exit condition)
        let condition = HirExpr::new(
            HirExprKind::BinOp {
                op: HirBinOp::Lt,
                lhs: Box::new(HirExpr::new(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    span,
                )),
                rhs: Box::new(HirExpr::new(
                    HirExprKind::Local {
                        name: internal_len.clone(),
                    },
                    span,
                )),
            },
            span,
        );

        // Build desugared block:
        // {
        //     let __arr = array
        //     let __len = __arr.len()
        //     let __idx = 0
        //     while __idx < __len { ... }
        // }
        let arr_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_arr.clone(),
                type_id: None,
                value: self.lower_expr(array_expr),
                mutable: false,
                ownership: Ownership::Owned,
            },
            span,
        );

        // Pre-compute array length to keep the loop condition pure (integer
        // comparison only). This prevents LLVM O3 from merging array-clone
        // code into the loop header and subsequently removing the exit branch.
        let len_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_len,
                type_id: None,
                value: HirExpr::new(
                    HirExprKind::MethodCall {
                        receiver: Box::new(HirExpr::new(
                            HirExprKind::Local {
                                name: internal_arr.clone(),
                            },
                            span,
                        )),
                        method: "len".to_string(),
                        args: vec![],
                    },
                    span,
                ),
                mutable: false,
                ownership: Ownership::Owned,
            },
            span,
        );

        let idx_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_idx,
                type_id: None,
                value: HirExpr::new(HirExprKind::Const(ConstValue::Int(0)), span),
                mutable: true,
                ownership: Ownership::Owned,
            },
            span,
        );

        let while_stmt = HirStmt::new(
            HirStmtKind::While {
                condition,
                body: body_stmts,
                increment: vec![increment_stmt],
            },
            span,
        );

        HirStmtKind::Expr(HirExpr::new(
            HirExprKind::Block {
                stmts: vec![arr_init, len_init, idx_init, while_stmt],
                expr: None,
            },
            span,
        ))
    }

    /// Lower map-based for-loop: `for key, value in map` or `for key in map`
    ///
    /// Desugars to:
    /// ```text
    /// let __map = map
    /// let __keys = __map.keys()
    /// let __idx = 0
    /// while __idx < __keys.len() {
    ///     let key = __keys[__idx]
    ///     let value = __map.get(key)
    ///     body
    ///     __idx = __idx + 1
    /// }
    /// ```
    pub(crate) fn lower_map_for_loop(
        &mut self,
        pattern: &Pattern,
        map_expr: &Expr,
        body: &[Stmt],
        span: Span,
    ) -> HirStmtKind {
        // Check if pattern is tuple (key, value) or single (key)
        let (key_var, value_var) = match &pattern.kind {
            PatternKind::Tuple(patterns) if patterns.len() == 2 => {
                let key = self.pattern_to_name(&patterns[0]);
                let val = self.pattern_to_name(&patterns[1]);
                (key, Some(val))
            }
            _ => (self.pattern_to_name(pattern), None),
        };

        // Generate unique internal variable names to avoid conflicts with multiple loops
        let uid = self.unique_suffix();
        let internal_map = format!("__{}map{}", key_var, uid);
        let internal_keys = format!("__{}keys{}", key_var, uid);
        let internal_idx = format!("__{}idx{}", key_var, uid);

        // Lower body statements
        let mut body_stmts: Vec<_> = body.iter().map(|s| self.lower_stmt(s)).collect();

        // Prepend: let key = __keys[__idx]
        body_stmts.insert(
            0,
            HirStmt::new(
                HirStmtKind::Let {
                    name: key_var.clone(),
                    type_id: None,
                    value: HirExpr::new(
                        HirExprKind::Index {
                            object: Box::new(HirExpr::new(
                                HirExprKind::Local {
                                    name: internal_keys.clone(),
                                },
                                span,
                            )),
                            index: Box::new(HirExpr::new(
                                HirExprKind::Local {
                                    name: internal_idx.clone(),
                                },
                                span,
                            )),
                        },
                        span,
                    ),
                    mutable: false,
                    ownership: Ownership::Owned,
                },
                span,
            ),
        );

        // Prepend: let value = __map.get(key) if we have a value variable
        if let Some(val_name) = &value_var {
            body_stmts.insert(
                1,
                HirStmt::new(
                    HirStmtKind::Let {
                        name: val_name.clone(),
                        type_id: None,
                        value: HirExpr::new(
                            HirExprKind::Index {
                                object: Box::new(HirExpr::new(
                                    HirExprKind::Local {
                                        name: internal_map.clone(),
                                    },
                                    span,
                                )),
                                index: Box::new(HirExpr::new(
                                    HirExprKind::Local {
                                        name: key_var.clone(),
                                    },
                                    span,
                                )),
                            },
                            span,
                        ),
                        mutable: false,
                        ownership: Ownership::Owned,
                    },
                    span,
                ),
            );
        }

        // Append: __idx = __idx + 1
        let increment_stmt = HirStmt::new(
            HirStmtKind::Assign {
                target: HirExpr::new(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    span,
                ),
                value: HirExpr::new(
                    HirExprKind::BinOp {
                        op: HirBinOp::Add,
                        lhs: Box::new(HirExpr::new(
                            HirExprKind::Local {
                                name: internal_idx.clone(),
                            },
                            span,
                        )),
                        rhs: Box::new(HirExpr::new(HirExprKind::Const(ConstValue::Int(1)), span)),
                    },
                    span,
                ),
            },
            span,
        );

        // Condition: __idx < __keys.len()
        let condition = HirExpr::new(
            HirExprKind::BinOp {
                op: HirBinOp::Lt,
                lhs: Box::new(HirExpr::new(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    span,
                )),
                rhs: Box::new(HirExpr::new(
                    HirExprKind::MethodCall {
                        receiver: Box::new(HirExpr::new(
                            HirExprKind::Local {
                                name: internal_keys.clone(),
                            },
                            span,
                        )),
                        method: "len".to_string(),
                        args: vec![],
                    },
                    span,
                )),
            },
            span,
        );

        // Build desugared block:
        // {
        //     let __map = map
        //     let __keys = __map.keys()
        //     let __idx = 0
        //     while __idx < __keys.len() { ... }
        // }
        let map_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_map.clone(),
                type_id: None,
                value: self.lower_expr(map_expr),
                mutable: false,
                ownership: Ownership::Owned,
            },
            span,
        );

        let keys_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_keys,
                type_id: None,
                value: HirExpr::new(
                    HirExprKind::MethodCall {
                        receiver: Box::new(HirExpr::new(
                            HirExprKind::Local { name: internal_map },
                            span,
                        )),
                        method: "keys".to_string(),
                        args: vec![],
                    },
                    span,
                ),
                mutable: false,
                ownership: Ownership::Owned,
            },
            span,
        );

        let idx_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_idx,
                type_id: None,
                value: HirExpr::new(HirExprKind::Const(ConstValue::Int(0)), span),
                mutable: true,
                ownership: Ownership::Owned,
            },
            span,
        );

        let while_stmt = HirStmt::new(
            HirStmtKind::While {
                condition,
                body: body_stmts,
                increment: vec![increment_stmt],
            },
            span,
        );

        HirStmtKind::Expr(HirExpr::new(
            HirExprKind::Block {
                stmts: vec![map_init, keys_init, idx_init, while_stmt],
                expr: None,
            },
            span,
        ))
    }

    /// Lower a for-loop with type information.
    pub(crate) fn lower_for_loop_typed(
        &mut self,
        pattern: &Pattern,
        iterable: Option<&Expr>,
        body: &[Stmt],
        span: Span,
        registry: &mut TypeRegistry,
    ) -> HirStmtKind {
        // No iterable = infinite loop
        let Some(iter_expr) = iterable else {
            let body_stmts: Vec<_> = body
                .iter()
                .map(|s| self.lower_stmt_typed(s, registry))
                .collect();
            return HirStmtKind::While {
                condition: HirExpr::with_type(
                    HirExprKind::Const(ConstValue::Bool(true)),
                    builtin::BOOL,
                    span,
                ),
                body: body_stmts,
                increment: vec![],
            };
        };

        // Check if iterating over a range expression
        if let ExprKind::Range {
            start,
            end,
            inclusive,
        } = &iter_expr.kind
        {
            return self
                .lower_range_for_loop_typed(pattern, start, end, *inclusive, body, span, registry);
        }

        // Check if iterating over a map - either by literal or by type
        // First check for map literal
        if matches!(&iter_expr.kind, ExprKind::MapLit(_)) {
            return self.lower_map_for_loop_typed(pattern, iter_expr, body, span, registry);
        }

        // Then check if the iterable expression has a Map type
        // We need to lower it first to get its type
        let lowered = self.lower_expr_typed(iter_expr, registry);
        let is_map = lowered.type_id.map_or(false, |tid| {
            registry
                .get(tid)
                .map_or(false, |info| matches!(info.kind, TypeKind::Map { .. }))
        });

        if is_map {
            // Re-lower using map-specific lowering
            // Since we already lowered, we can use that result
            return self
                .lower_map_for_loop_typed_with_lowered(pattern, lowered, body, span, registry);
        }

        // Array/collection iteration
        self.lower_array_for_loop_typed(pattern, iter_expr, body, span, registry)
    }

    /// Lower range-based for-loop with type info: `for i in start..end`
    pub(crate) fn lower_range_for_loop_typed(
        &mut self,
        pattern: &Pattern,
        start: &Expr,
        end: &Expr,
        inclusive: bool,
        body: &[Stmt],
        span: Span,
        registry: &mut TypeRegistry,
    ) -> HirStmtKind {
        let iter_var = self.pattern_to_name(pattern);
        let internal_idx = format!("__{}_idx", iter_var);

        // Lower body statements
        let mut body_stmts: Vec<_> = body
            .iter()
            .map(|s| self.lower_stmt_typed(s, registry))
            .collect();

        // Prepend: let iter_var = __idx
        body_stmts.insert(
            0,
            HirStmt::new(
                HirStmtKind::Let {
                    name: iter_var.clone(),
                    type_id: Some(builtin::INT),
                    value: HirExpr::with_type(
                        HirExprKind::Local {
                            name: internal_idx.clone(),
                        },
                        builtin::INT,
                        span,
                    ),
                    mutable: false,
                    ownership: Ownership::Owned,
                },
                span,
            ),
        );

        // Append: __idx = __idx + 1
        let increment_stmt = HirStmt::new(
            HirStmtKind::Assign {
                target: HirExpr::with_type(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    builtin::INT,
                    span,
                ),
                value: HirExpr::with_type(
                    HirExprKind::BinOp {
                        op: HirBinOp::Add,
                        lhs: Box::new(HirExpr::with_type(
                            HirExprKind::Local {
                                name: internal_idx.clone(),
                            },
                            builtin::INT,
                            span,
                        )),
                        rhs: Box::new(HirExpr::with_type(
                            HirExprKind::Const(ConstValue::Int(1)),
                            builtin::INT,
                            span,
                        )),
                    },
                    builtin::INT,
                    span,
                ),
            },
            span,
        );

        // Condition: __idx < end (or __idx <= end for inclusive)
        let cmp_op = if inclusive {
            HirBinOp::LtEq
        } else {
            HirBinOp::Lt
        };
        let condition = HirExpr::with_type(
            HirExprKind::BinOp {
                op: cmp_op,
                lhs: Box::new(HirExpr::with_type(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    builtin::INT,
                    span,
                )),
                rhs: Box::new(self.lower_expr_typed(end, registry)),
            },
            builtin::BOOL,
            span,
        );

        // Build desugared block
        let init_stmt = HirStmt::new(
            HirStmtKind::Let {
                name: internal_idx,
                type_id: Some(builtin::INT),
                value: self.lower_expr_typed(start, registry),
                mutable: true,
                ownership: Ownership::Owned,
            },
            span,
        );

        let while_stmt = HirStmt::new(
            HirStmtKind::While {
                condition,
                body: body_stmts,
                increment: vec![increment_stmt],
            },
            span,
        );

        HirStmtKind::Expr(HirExpr::new(
            HirExprKind::Block {
                stmts: vec![init_stmt, while_stmt],
                expr: None,
            },
            span,
        ))
    }

    /// Lower array-based for-loop with type info: `for x in array` or `for i, x in array`
    pub(crate) fn lower_array_for_loop_typed(
        &mut self,
        pattern: &Pattern,
        array_expr: &Expr,
        body: &[Stmt],
        span: Span,
        registry: &mut TypeRegistry,
    ) -> HirStmtKind {
        // Check if pattern is tuple (i, x) or single (x)
        let (index_var, elem_var) = match &pattern.kind {
            PatternKind::Tuple(patterns) if patterns.len() == 2 => {
                let idx = self.pattern_to_name(&patterns[0]);
                let elem = self.pattern_to_name(&patterns[1]);
                (Some(idx), elem)
            }
            _ => (None, self.pattern_to_name(pattern)),
        };

        let internal_idx = format!("__{}_idx", elem_var);
        let internal_arr = format!("__{}_arr", elem_var);
        let internal_len = format!("__{}_len", elem_var);

        // Lower the array expression to get its type
        let lowered_arr = self.lower_expr_typed(array_expr, registry);
        let arr_type = lowered_arr.type_id;

        // Infer element type from array type
        let elem_type = arr_type.and_then(|tid| {
            registry.get(tid).and_then(|info| match &info.kind {
                TypeKind::Array { element } => Some(*element),
                _ => None,
            })
        });

        // Lower body statements
        let mut body_stmts: Vec<_> = body
            .iter()
            .map(|s| self.lower_stmt_typed(s, registry))
            .collect();

        // Prepend index assignment if pattern has index: let i = __idx
        if let Some(idx_name) = &index_var {
            body_stmts.insert(
                0,
                HirStmt::new(
                    HirStmtKind::Let {
                        name: idx_name.clone(),
                        type_id: Some(builtin::INT),
                        value: HirExpr::with_type(
                            HirExprKind::Local {
                                name: internal_idx.clone(),
                            },
                            builtin::INT,
                            span,
                        ),
                        mutable: false,
                        ownership: Ownership::Owned,
                    },
                    span,
                ),
            );
        }

        // Prepend element extraction: let x = __arr[__idx]
        // Create array reference with proper type
        let arr_ref = if let Some(t) = arr_type {
            HirExpr::with_type(
                HirExprKind::Local {
                    name: internal_arr.clone(),
                },
                t,
                span,
            )
        } else {
            HirExpr::new(
                HirExprKind::Local {
                    name: internal_arr.clone(),
                },
                span,
            )
        };

        // Create index expression with proper element type
        let index_expr = if let Some(t) = elem_type {
            HirExpr::with_type(
                HirExprKind::Index {
                    object: Box::new(arr_ref.clone()),
                    index: Box::new(HirExpr::with_type(
                        HirExprKind::Local {
                            name: internal_idx.clone(),
                        },
                        builtin::INT,
                        span,
                    )),
                },
                t,
                span,
            )
        } else {
            HirExpr::new(
                HirExprKind::Index {
                    object: Box::new(arr_ref.clone()),
                    index: Box::new(HirExpr::with_type(
                        HirExprKind::Local {
                            name: internal_idx.clone(),
                        },
                        builtin::INT,
                        span,
                    )),
                },
                span,
            )
        };

        let elem_extraction = HirStmt::new(
            HirStmtKind::Let {
                name: elem_var.clone(),
                type_id: elem_type,
                value: index_expr,
                mutable: false,
                ownership: Ownership::Owned,
            },
            span,
        );
        let insert_pos = if index_var.is_some() { 1 } else { 0 };
        body_stmts.insert(insert_pos, elem_extraction);

        // Append: __idx = __idx + 1
        let increment_stmt = HirStmt::new(
            HirStmtKind::Assign {
                target: HirExpr::with_type(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    builtin::INT,
                    span,
                ),
                value: HirExpr::with_type(
                    HirExprKind::BinOp {
                        op: HirBinOp::Add,
                        lhs: Box::new(HirExpr::with_type(
                            HirExprKind::Local {
                                name: internal_idx.clone(),
                            },
                            builtin::INT,
                            span,
                        )),
                        rhs: Box::new(HirExpr::with_type(
                            HirExprKind::Const(ConstValue::Int(1)),
                            builtin::INT,
                            span,
                        )),
                    },
                    builtin::INT,
                    span,
                ),
            },
            span,
        );

        // Condition: __idx < __len (pre-computed length, avoids array access
        // in the loop header which triggers auto-cloning and generates complex
        // LLVM IR that causes O3 to remove the loop exit condition)
        let condition = HirExpr::with_type(
            HirExprKind::BinOp {
                op: HirBinOp::Lt,
                lhs: Box::new(HirExpr::with_type(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    builtin::INT,
                    span,
                )),
                rhs: Box::new(HirExpr::with_type(
                    HirExprKind::Local {
                        name: internal_len.clone(),
                    },
                    builtin::INT,
                    span,
                )),
            },
            builtin::BOOL,
            span,
        );

        // Build desugared block
        let arr_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_arr.clone(),
                type_id: arr_type,
                value: lowered_arr,
                mutable: false,
                ownership: Ownership::Owned,
            },
            span,
        );

        // Pre-compute array length to keep the loop condition pure (integer
        // comparison only). This prevents LLVM O3 from merging array-clone
        // code into the loop header and subsequently removing the exit branch.
        let arr_ref_for_len = if let Some(t) = arr_type {
            HirExpr::with_type(
                HirExprKind::Local {
                    name: internal_arr.clone(),
                },
                t,
                span,
            )
        } else {
            HirExpr::new(
                HirExprKind::Local {
                    name: internal_arr.clone(),
                },
                span,
            )
        };

        let len_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_len,
                type_id: Some(builtin::INT),
                value: HirExpr::with_type(
                    HirExprKind::MethodCall {
                        receiver: Box::new(arr_ref_for_len),
                        method: "len".to_string(),
                        args: vec![],
                    },
                    builtin::INT,
                    span,
                ),
                mutable: false,
                ownership: Ownership::Owned,
            },
            span,
        );

        let idx_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_idx,
                type_id: Some(builtin::INT),
                value: HirExpr::with_type(
                    HirExprKind::Const(ConstValue::Int(0)),
                    builtin::INT,
                    span,
                ),
                mutable: true,
                ownership: Ownership::Owned,
            },
            span,
        );

        let while_stmt = HirStmt::new(
            HirStmtKind::While {
                condition,
                body: body_stmts,
                increment: vec![increment_stmt],
            },
            span,
        );

        HirStmtKind::Expr(HirExpr::new(
            HirExprKind::Block {
                stmts: vec![arr_init, len_init, idx_init, while_stmt],
                expr: None,
            },
            span,
        ))
    }

    /// Lower map-based for-loop with type info: `for key, value in map` or `for key in map`
    pub(crate) fn lower_map_for_loop_typed(
        &mut self,
        pattern: &Pattern,
        map_expr: &Expr,
        body: &[Stmt],
        span: Span,
        registry: &mut TypeRegistry,
    ) -> HirStmtKind {
        // Check if pattern is tuple (key, value) or single (key)
        let (orig_key_var, orig_value_var) = match &pattern.kind {
            PatternKind::Tuple(patterns) if patterns.len() == 2 => {
                let key = self.pattern_to_name(&patterns[0]);
                let val = self.pattern_to_name(&patterns[1]);
                (key, Some(val))
            }
            _ => (self.pattern_to_name(pattern), None),
        };

        // Generate unique internal variable names to avoid conflicts with multiple loops
        let uid = self.unique_suffix();
        let internal_map = format!("__{}map{}", orig_key_var, uid);
        let internal_keys = format!("__{}keys{}", orig_key_var, uid);
        let internal_idx = format!("__{}idx{}", orig_key_var, uid);

        // Also make the iteration variables unique to avoid type conflicts
        // Use k/v prefixes so key and value don't collide when both are _ (wildcard)
        let key_var = format!("__k{}_{}", orig_key_var, uid);
        let value_var = orig_value_var.as_ref().map(|v| format!("__v{}_{}", v, uid));

        // Lower the map expression FIRST to get its type for proper propagation in body
        let lowered_map_early = self.lower_expr_typed(map_expr, registry);
        let map_type_early = lowered_map_early.type_id;

        // Lower body statements, substituting the original variable names with unique ones
        let mut body_stmts: Vec<_> = body
            .iter()
            .map(|s| {
                let mut lowered = self.lower_stmt_typed(s, registry);
                // Substitute variable references
                self.substitute_local_in_stmt(&mut lowered, &orig_key_var, &key_var);
                if let (Some(ref orig_val), Some(ref new_val)) = (&orig_value_var, &value_var) {
                    self.substitute_local_in_stmt(&mut lowered, orig_val, new_val);
                }
                lowered
            })
            .collect();

        // Prepend: let key = __keys[__idx]
        body_stmts.insert(
            0,
            HirStmt::new(
                HirStmtKind::Let {
                    name: key_var.clone(),
                    type_id: None,
                    value: HirExpr::new(
                        HirExprKind::Index {
                            object: Box::new(HirExpr::new(
                                HirExprKind::Local {
                                    name: internal_keys.clone(),
                                },
                                span,
                            )),
                            index: Box::new(HirExpr::with_type(
                                HirExprKind::Local {
                                    name: internal_idx.clone(),
                                },
                                builtin::INT,
                                span,
                            )),
                        },
                        span,
                    ),
                    mutable: false,
                    ownership: Ownership::Owned,
                },
                span,
            ),
        );

        // Prepend: let value = __map[key] if we have a value variable
        if let Some(val_name) = &value_var {
            body_stmts.insert(
                1,
                HirStmt::new(
                    HirStmtKind::Let {
                        name: val_name.clone(),
                        type_id: None,
                        value: HirExpr::new(
                            HirExprKind::Index {
                                object: Box::new(match map_type_early {
                                    Some(t) => HirExpr::with_type(
                                        HirExprKind::Local {
                                            name: internal_map.clone(),
                                        },
                                        t,
                                        span,
                                    ),
                                    None => HirExpr::new(
                                        HirExprKind::Local {
                                            name: internal_map.clone(),
                                        },
                                        span,
                                    ),
                                }),
                                index: Box::new(HirExpr::new(
                                    HirExprKind::Local {
                                        name: key_var.clone(),
                                    },
                                    span,
                                )),
                            },
                            span,
                        ),
                        mutable: false,
                        ownership: Ownership::Owned,
                    },
                    span,
                ),
            );
        }

        // Append: __idx = __idx + 1
        let increment_stmt = HirStmt::new(
            HirStmtKind::Assign {
                target: HirExpr::with_type(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    builtin::INT,
                    span,
                ),
                value: HirExpr::with_type(
                    HirExprKind::BinOp {
                        op: HirBinOp::Add,
                        lhs: Box::new(HirExpr::with_type(
                            HirExprKind::Local {
                                name: internal_idx.clone(),
                            },
                            builtin::INT,
                            span,
                        )),
                        rhs: Box::new(HirExpr::with_type(
                            HirExprKind::Const(ConstValue::Int(1)),
                            builtin::INT,
                            span,
                        )),
                    },
                    builtin::INT,
                    span,
                ),
            },
            span,
        );

        // Condition: __idx < __keys.len()
        let condition = HirExpr::with_type(
            HirExprKind::BinOp {
                op: HirBinOp::Lt,
                lhs: Box::new(HirExpr::with_type(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    builtin::INT,
                    span,
                )),
                rhs: Box::new(HirExpr::with_type(
                    HirExprKind::MethodCall {
                        receiver: Box::new(HirExpr::new(
                            HirExprKind::Local {
                                name: internal_keys.clone(),
                            },
                            span,
                        )),
                        method: "len".to_string(),
                        args: vec![],
                    },
                    builtin::INT,
                    span,
                )),
            },
            builtin::BOOL,
            span,
        );

        // Build desugared block - use the already lowered map expression
        let map_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_map.clone(),
                type_id: map_type_early,
                value: lowered_map_early.clone(),
                mutable: false,
                ownership: Ownership::Owned,
            },
            span,
        );

        let keys_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_keys.clone(),
                type_id: None,
                value: {
                    // CRITICAL: The receiver must have the map type set for proper type propagation
                    let receiver = match map_type_early {
                        Some(t) => HirExpr::with_type(
                            HirExprKind::Local {
                                name: internal_map.clone(),
                            },
                            t,
                            span,
                        ),
                        None => HirExpr::new(
                            HirExprKind::Local {
                                name: internal_map.clone(),
                            },
                            span,
                        ),
                    };
                    let keys_call = HirExpr::new(
                        HirExprKind::MethodCall {
                            receiver: Box::new(receiver),
                            method: "keys".to_string(),
                            args: vec![],
                        },
                        span,
                    );

                    // Infer the return type of keys() method
                    if let Some(map_ty) = map_type_early {
                        if let Some(keys_ty) =
                            self.infer_method_call_type(map_ty, "keys", &mut [], registry)
                        {
                            HirExpr::with_type(keys_call.kind, keys_ty, span)
                        } else {
                            keys_call
                        }
                    } else {
                        keys_call
                    }
                },
                mutable: false,
                ownership: Ownership::Owned,
            },
            span,
        );

        let idx_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_idx,
                type_id: Some(builtin::INT),
                value: HirExpr::with_type(
                    HirExprKind::Const(ConstValue::Int(0)),
                    builtin::INT,
                    span,
                ),
                mutable: true,
                ownership: Ownership::Owned,
            },
            span,
        );

        let while_stmt = HirStmt::new(
            HirStmtKind::While {
                condition,
                body: body_stmts,
                increment: vec![increment_stmt],
            },
            span,
        );

        HirStmtKind::Expr(HirExpr::new(
            HirExprKind::Block {
                stmts: vec![map_init, keys_init, idx_init, while_stmt],
                expr: None,
            },
            span,
        ))
    }

    /// Lower map-based for-loop with an already-lowered map expression
    /// This variant is used when we detect the map type after lowering the expression
    pub(crate) fn lower_map_for_loop_typed_with_lowered(
        &mut self,
        pattern: &Pattern,
        lowered_map: HirExpr,
        body: &[Stmt],
        span: Span,
        registry: &mut TypeRegistry,
    ) -> HirStmtKind {
        // Check if pattern is tuple (key, value) or single (key)
        let (orig_key_var, orig_value_var) = match &pattern.kind {
            PatternKind::Tuple(patterns) if patterns.len() == 2 => {
                let key = self.pattern_to_name(&patterns[0]);
                let val = self.pattern_to_name(&patterns[1]);
                (key, Some(val))
            }
            _ => (self.pattern_to_name(pattern), None),
        };

        // Generate unique internal variable names to avoid conflicts with multiple loops
        let uid = self.unique_suffix();
        let internal_map = format!("__{}map{}", orig_key_var, uid);
        let internal_keys = format!("__{}keys{}", orig_key_var, uid);
        let internal_idx = format!("__{}idx{}", orig_key_var, uid);

        // Also make the iteration variables unique to avoid type conflicts
        // Use k/v prefixes so key and value don't collide when both are _ (wildcard)
        let key_var = format!("__k{}_{}", orig_key_var, uid);
        let value_var = orig_value_var.as_ref().map(|v| format!("__v{}_{}", v, uid));

        // Get the map type from the already-lowered map expression
        let map_type = lowered_map.type_id;

        // Lower body statements, substituting the original variable names with unique ones
        let mut body_stmts: Vec<_> = body
            .iter()
            .map(|s| {
                let mut lowered = self.lower_stmt_typed(s, registry);
                // Substitute variable references
                self.substitute_local_in_stmt(&mut lowered, &orig_key_var, &key_var);
                if let (Some(ref orig_val), Some(ref new_val)) = (&orig_value_var, &value_var) {
                    self.substitute_local_in_stmt(&mut lowered, orig_val, new_val);
                }
                lowered
            })
            .collect();

        // Prepend: let key = __keys[__idx]
        body_stmts.insert(
            0,
            HirStmt::new(
                HirStmtKind::Let {
                    name: key_var.clone(),
                    type_id: None,
                    value: HirExpr::new(
                        HirExprKind::Index {
                            object: Box::new(HirExpr::new(
                                HirExprKind::Local {
                                    name: internal_keys.clone(),
                                },
                                span,
                            )),
                            index: Box::new(HirExpr::with_type(
                                HirExprKind::Local {
                                    name: internal_idx.clone(),
                                },
                                builtin::INT,
                                span,
                            )),
                        },
                        span,
                    ),
                    mutable: false,
                    ownership: Ownership::Owned,
                },
                span,
            ),
        );

        // Prepend: let value = __map[key] if we have a value variable
        if let Some(val_name) = &value_var {
            body_stmts.insert(
                1,
                HirStmt::new(
                    HirStmtKind::Let {
                        name: val_name.clone(),
                        type_id: None,
                        value: HirExpr::new(
                            HirExprKind::Index {
                                object: Box::new(match map_type {
                                    Some(t) => HirExpr::with_type(
                                        HirExprKind::Local {
                                            name: internal_map.clone(),
                                        },
                                        t,
                                        span,
                                    ),
                                    None => HirExpr::new(
                                        HirExprKind::Local {
                                            name: internal_map.clone(),
                                        },
                                        span,
                                    ),
                                }),
                                index: Box::new(HirExpr::new(
                                    HirExprKind::Local {
                                        name: key_var.clone(),
                                    },
                                    span,
                                )),
                            },
                            span,
                        ),
                        mutable: false,
                        ownership: Ownership::Owned,
                    },
                    span,
                ),
            );
        }

        // Append: __idx = __idx + 1
        let increment_stmt = HirStmt::new(
            HirStmtKind::Assign {
                target: HirExpr::with_type(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    builtin::INT,
                    span,
                ),
                value: HirExpr::with_type(
                    HirExprKind::BinOp {
                        op: HirBinOp::Add,
                        lhs: Box::new(HirExpr::with_type(
                            HirExprKind::Local {
                                name: internal_idx.clone(),
                            },
                            builtin::INT,
                            span,
                        )),
                        rhs: Box::new(HirExpr::with_type(
                            HirExprKind::Const(ConstValue::Int(1)),
                            builtin::INT,
                            span,
                        )),
                    },
                    builtin::INT,
                    span,
                ),
            },
            span,
        );

        // Condition: __idx < __keys.len()
        let condition = HirExpr::with_type(
            HirExprKind::BinOp {
                op: HirBinOp::Lt,
                lhs: Box::new(HirExpr::with_type(
                    HirExprKind::Local {
                        name: internal_idx.clone(),
                    },
                    builtin::INT,
                    span,
                )),
                rhs: Box::new(HirExpr::with_type(
                    HirExprKind::MethodCall {
                        receiver: Box::new(HirExpr::new(
                            HirExprKind::Local {
                                name: internal_keys.clone(),
                            },
                            span,
                        )),
                        method: "len".to_string(),
                        args: vec![],
                    },
                    builtin::INT,
                    span,
                )),
            },
            builtin::BOOL,
            span,
        );

        // Build desugared block using the already-lowered map expression
        let map_type = lowered_map.type_id;
        let map_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_map.clone(),
                type_id: map_type,
                value: lowered_map.clone(),
                mutable: false,
                ownership: Ownership::Owned,
            },
            span,
        );

        let keys_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_keys.clone(),
                type_id: None,
                value: {
                    // CRITICAL: The receiver must have the map type set for proper type propagation
                    let receiver = match map_type {
                        Some(t) => HirExpr::with_type(
                            HirExprKind::Local {
                                name: internal_map.clone(),
                            },
                            t,
                            span,
                        ),
                        None => HirExpr::new(
                            HirExprKind::Local {
                                name: internal_map.clone(),
                            },
                            span,
                        ),
                    };
                    let keys_call = HirExpr::new(
                        HirExprKind::MethodCall {
                            receiver: Box::new(receiver),
                            method: "keys".to_string(),
                            args: vec![],
                        },
                        span,
                    );

                    // Infer the return type of keys() method
                    if let Some(map_ty) = map_type {
                        if let Some(keys_ty) =
                            self.infer_method_call_type(map_ty, "keys", &mut [], registry)
                        {
                            HirExpr::with_type(keys_call.kind, keys_ty, span)
                        } else {
                            keys_call
                        }
                    } else {
                        keys_call
                    }
                },
                mutable: false,
                ownership: Ownership::Owned,
            },
            span,
        );

        let idx_init = HirStmt::new(
            HirStmtKind::Let {
                name: internal_idx,
                type_id: Some(builtin::INT),
                value: HirExpr::with_type(
                    HirExprKind::Const(ConstValue::Int(0)),
                    builtin::INT,
                    span,
                ),
                mutable: true,
                ownership: Ownership::Owned,
            },
            span,
        );

        let while_stmt = HirStmt::new(
            HirStmtKind::While {
                condition,
                body: body_stmts,
                increment: vec![increment_stmt],
            },
            span,
        );

        HirStmtKind::Expr(HirExpr::new(
            HirExprKind::Block {
                stmts: vec![map_init, keys_init, idx_init, while_stmt],
                expr: None,
            },
            span,
        ))
    }
}
