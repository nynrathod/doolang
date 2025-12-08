use crate::{
    lexer::token::TokenType,
    mir::statements::build_statement,
    mir::{builder::MirBuilder, MirBlock, MirInstr},
    parser::ast::{AstNode, TypeNode},
};

/// Helper function to get the variant index for an enum variant
/// Returns 0 if the enum or variant is not found
fn get_enum_variant_index(builder: &MirBuilder, enum_name: &str, variant_name: &str) -> u32 {
    // First try to use enum_variant_order which preserves declaration order
    if let Some(variants) = builder.enum_variant_order.get(enum_name) {
        for (idx, (name, _)) in variants.iter().enumerate() {
            if name == variant_name {
                return idx as u32;
            }
        }
    }
    // Fallback: use enum_table with sorted keys (for determinism, not order preservation)
    if let Some(variants) = builder.enum_table.get(enum_name) {
        let mut sorted_variants: Vec<_> = variants.keys().collect();
        sorted_variants.sort();
        for (idx, name) in sorted_variants.iter().enumerate() {
            if *name == variant_name {
                return idx as u32;
            }
        }
    }
    0
}

/// Helper function to determine if a method mutates its receiver
/// These methods need special handling for struct fields
fn is_mutating_method(method: &str) -> bool {
    matches!(method, "push" | "pop" | "clear" | "insert" | "remove")
}

/// Helper function to determine the type of an operand by looking it up in the symbol table
/// If not found, tries to infer the type from the operand value (e.g., literals)
/// Helper function to convert TypeNode to source_type string for Cast instructions
fn type_node_to_source_type(ty: &Option<TypeNode>) -> String {
    match ty {
        Some(TypeNode::Float) => "Float".to_string(),
        Some(TypeNode::Int) => "Int".to_string(),
        Some(TypeNode::Bool) => "Bool".to_string(),
        Some(TypeNode::String) => "String".to_string(),
        Some(TypeNode::Array(inner)) => format!("Array({})", inner.format_type_string()),
        Some(TypeNode::Map(k, v)) => {
            format!("Map({},{})", k.format_type_string(), v.format_type_string())
        }
        Some(TypeNode::Struct(name, _)) => format!("Struct({})", name),
        Some(TypeNode::TypeRef(name)) => format!("Struct({})", name),
        Some(TypeNode::Enum(name, _)) => format!("Enum({})", name),
        Some(TypeNode::Tuple(types)) => {
            let parts: Vec<String> = types.iter().map(|t| t.format_type_string()).collect();
            format!("Tuple({})", parts.join(","))
        }
        Some(TypeNode::Result(ok, err)) => format!(
            "Result({},{})",
            ok.format_type_string(),
            err.format_type_string()
        ),
        Some(TypeNode::Any) => "Any".to_string(),
        _ => "Int".to_string(), // Default fallback
    }
}

fn get_operand_type(builder: &MirBuilder, operand: &str) -> Option<TypeNode> {
    // Check for nil first - it's a special pointer value
    if operand == "nil" {
        return Some(TypeNode::String); // Using String as a marker for "pointer type"
    }

    // First try to look up in symbol table
    if let Some(ty) = builder.mir_symbol_table.get(operand).cloned() {
        return Some(ty);
    }

    // Try to infer type from the operand itself
    // Check if it's a float literal (contains a dot)
    if operand.contains('.') {
        if let Ok(_) = operand.parse::<f64>() {
            return Some(TypeNode::Float);
        }
    }

    // Check if it's an integer literal
    if let Ok(_) = operand.parse::<i32>() {
        return Some(TypeNode::Int);
    }

    // Check if it's a boolean literal
    if operand == "true" || operand == "false" {
        return Some(TypeNode::Bool);
    }

    None
}

/// Helper function to determine the operation type for binary operations
/// Returns "float" if either operand is float, "int" if both are int, or None for incompatible types
pub fn determine_op_type(builder: &MirBuilder, lhs: &str, rhs: &str) -> Result<String, String> {
    let lhs_type = get_operand_type(builder, lhs);
    let rhs_type = get_operand_type(builder, rhs);

    // Handle nil comparisons specially - nil can be compared with any pointer type
    // nil is represented as String type in get_operand_type
    let is_nil_comparison = (lhs == "nil" || rhs == "nil")
        || (lhs.starts_with("%") && rhs == "nil")
        || (rhs.starts_with("%") && lhs == "nil");

    if is_nil_comparison {
        // For nil comparisons, use pointer comparison (int type in LLVM)
        return Ok("int".to_string());
    }

    match (&lhs_type, &rhs_type) {
        (Some(TypeNode::Float), Some(TypeNode::Float)) => Ok("float".to_string()),
        (Some(TypeNode::Float), Some(TypeNode::Int)) => Ok("float".to_string()),
        (Some(TypeNode::Int), Some(TypeNode::Float)) => Ok("float".to_string()),
        (Some(TypeNode::Int), Some(TypeNode::Int)) => Ok("int".to_string()),
        (Some(TypeNode::Bool), Some(TypeNode::Bool)) => Ok("bool".to_string()),
        (Some(TypeNode::String), Some(TypeNode::String)) => Ok("string".to_string()),
        // Special case: None + String likely means error variable vs nil comparison
        // This happens because error variables from ManualErrorExtract aren't in symbol table yet
        (None, Some(TypeNode::String)) | (Some(TypeNode::String), None) => {
            // If one operand is unknown (likely an error variable) and the other is String (likely nil),
            // treat this as a pointer comparison
            Ok("int".to_string())
        }
        (Some(TypeNode::String), _) | (_, Some(TypeNode::String)) => {
            Err(format!("Cannot perform arithmetic on string types"))
        }
        // Support array comparisons if element types match
        (Some(TypeNode::Array(lhs_elem)), Some(TypeNode::Array(rhs_elem))) => {
            if lhs_elem == rhs_elem {
                Ok("array".to_string())
            } else {
                Err(format!(
                    "Type mismatch: cannot compare Array({:?}) with Array({:?})",
                    lhs_elem, rhs_elem
                ))
            }
        }
        // Support map comparisons if key and value types match
        (Some(TypeNode::Map(lhs_key, lhs_val)), Some(TypeNode::Map(rhs_key, rhs_val))) => {
            if lhs_key == rhs_key && lhs_val == rhs_val {
                Ok("map".to_string())
            } else {
                Err(format!(
                    "Type mismatch: cannot compare Map types with different key/value types"
                ))
            }
        }
        (Some(lhs_t), Some(rhs_t)) => Err(format!(
            "Type mismatch: cannot operate on {:?} and {:?}",
            lhs_t, rhs_t
        )),
        (None, None) => {
            // If we don't know both types, assume int (most common case)
            // This also handles pointer comparisons (struct == nil, etc.)
            Ok("int".to_string())
        }
        _ => {
            // One type is unknown - assume int (most common case)
            // This handles cases like err != nil where err type is not in symbol table yet
            // If it's actually a float operation, the codegen will handle it
            Ok("int".to_string())
        }
    }
}

pub fn build_expression(builder: &mut MirBuilder, expr: &AstNode, block: &mut MirBlock) -> String {
    // Check recursion depth to prevent stack overflow
    builder.recursion_depth += 1;
    if builder.recursion_depth > crate::limits::MIR_MAX_DEPTH {
        builder.recursion_depth -= 1;
        // Return error marker on depth exceeded
        let tmp = builder.next_tmp();
        block.instrs.push(MirInstr::ConstInt {
            name: tmp.clone(),
            value: 0,
        });
        return tmp;
    }

    let result = match expr {
        AstNode::NumberLiteral(n) => {
            let tmp = builder.next_tmp();
            block.instrs.push(MirInstr::ConstInt {
                name: tmp.clone(),
                value: *n,
            });
            // Track type in symbol table
            builder.mir_symbol_table.insert(tmp.clone(), TypeNode::Int);
            tmp
        }
        AstNode::FloatLiteral(f) => {
            let tmp = builder.next_tmp();
            block.instrs.push(MirInstr::ConstFloat {
                name: tmp.clone(),
                value: *f,
            });
            // Track type in symbol table
            builder
                .mir_symbol_table
                .insert(tmp.clone(), TypeNode::Float);
            tmp
        }

        AstNode::BoolLiteral(b) => {
            let tmp = builder.next_tmp();
            block.instrs.push(MirInstr::ConstBool {
                name: tmp.clone(),
                value: *b,
            });
            // Track type in symbol table
            builder.mir_symbol_table.insert(tmp.clone(), TypeNode::Bool);
            tmp
        }

        AstNode::NilLiteral => {
            let tmp = builder.next_tmp();
            block.instrs.push(MirInstr::ConstInt {
                name: tmp.clone(),
                value: 0, // Nil is represented as null pointer (0)
            });
            // Track type in symbol table as String (null pointer type)
            builder
                .mir_symbol_table
                .insert(tmp.clone(), TypeNode::String);
            tmp
        }

        AstNode::StringLiteral(s) => {
            let tmp = builder.next_tmp();
            block.instrs.push(MirInstr::ConstString {
                name: tmp.clone(),
                value: s.clone(),
            });
            // Track type in symbol table
            builder
                .mir_symbol_table
                .insert(tmp.clone(), TypeNode::String);
            tmp
        }

        AstNode::Identifier(name) => name.clone(),

        AstNode::UnaryExpr { op, expr } => {
            let expr_tmp = build_expression(builder, expr, block);
            let tmp = builder.next_tmp();

            match op {
                TokenType::Minus => {
                    // Negation: negate the operand
                    // Create a negate operation (0 - expr)
                    let zero_tmp = builder.next_tmp();

                    // Determine operation type based on operand
                    let op_type =
                        if let Some(TypeNode::Float) = builder.mir_symbol_table.get(&expr_tmp) {
                            "float".to_string()
                        } else {
                            "int".to_string()
                        };

                    // Create zero constant with the right type
                    if op_type == "float" {
                        block.instrs.push(MirInstr::ConstFloat {
                            name: zero_tmp.clone(),
                            value: 0.0,
                        });
                        builder
                            .mir_symbol_table
                            .insert(zero_tmp.clone(), TypeNode::Float);
                    } else {
                        block.instrs.push(MirInstr::ConstInt {
                            name: zero_tmp.clone(),
                            value: 0,
                        });
                        builder
                            .mir_symbol_table
                            .insert(zero_tmp.clone(), TypeNode::Int);
                    }

                    block.instrs.push(MirInstr::BinaryOp(
                        format!("sub:{}", op_type),
                        tmp.clone(),
                        zero_tmp,
                        expr_tmp.clone(),
                    ));

                    // Track result type
                    if let Some(expr_type) = builder.mir_symbol_table.get(&expr_tmp) {
                        builder
                            .mir_symbol_table
                            .insert(tmp.clone(), expr_type.clone());
                    } else {
                        builder.mir_symbol_table.insert(tmp.clone(), TypeNode::Int);
                    }

                    tmp
                }
                TokenType::Bang => {
                    // Logical NOT: !expr
                    // Implement as: expr != true (or expr == false)
                    let true_tmp = builder.next_tmp();
                    block.instrs.push(MirInstr::ConstBool {
                        name: true_tmp.clone(),
                        value: true,
                    });
                    builder
                        .mir_symbol_table
                        .insert(true_tmp.clone(), TypeNode::Bool);

                    block.instrs.push(MirInstr::BinaryOp(
                        "ne:bool".to_string(),
                        tmp.clone(),
                        expr_tmp,
                        true_tmp,
                    ));
                    builder.mir_symbol_table.insert(tmp.clone(), TypeNode::Bool);
                    tmp
                }
                _ => {
                    debug_assert!(
                        false,
                        "Unsupported unary operator: {:?} - should be caught by analyzer",
                        op
                    );
                    String::new() // Fallback for release builds
                }
            }
        }

        AstNode::BinaryExpr { left, op, right } => {
            // Special handling for "in" operator (key in map or element in array)
            if *op == TokenType::In {
                let left_tmp = build_expression(builder, left, block);
                let right_tmp = build_expression(builder, right, block);
                let result_tmp = builder.next_tmp();

                // Check if right side is an array or map based on type
                let right_type = get_operand_type(builder, &right_tmp);

                if let Some(TypeNode::Array(_)) = right_type {
                    // Array contains check
                    block.instrs.push(MirInstr::ArrayContains {
                        name: result_tmp.clone(),
                        array: right_tmp,
                        element: left_tmp,
                    });
                } else {
                    // Map contains check (default)
                    block.instrs.push(MirInstr::MapContains {
                        name: result_tmp.clone(),
                        map: right_tmp,
                        key: left_tmp,
                    });
                }

                builder
                    .mir_symbol_table
                    .insert(result_tmp.clone(), TypeNode::Bool);

                return result_tmp;
            }

            // Special handling for range expressions (.., ..=) used in for loops.
            match op {
                TokenType::RangeExc | TokenType::RangeInc => {
                    let start_tmp = build_expression(builder, left, block);
                    let end_tmp = build_expression(builder, right, block);
                    let range_tmp = builder.next_tmp();

                    block.instrs.push(MirInstr::RangeCreate {
                        name: range_tmp.clone(),
                        start: start_tmp,
                        end: end_tmp,
                        inclusive: matches!(op, TokenType::RangeInc),
                    });

                    range_tmp
                }

                _ => {
                    // Regular binary operations (add, sub, mul, div, etc.).
                    let lhs_tmp = build_expression(builder, left, block);
                    let rhs_tmp = build_expression(builder, right, block);
                    let dest_tmp = builder.next_tmp();

                    if *op == TokenType::Plus {
                        // Check if this is string concatenation
                        let lhs_type = get_operand_type(builder, &lhs_tmp);
                        let rhs_type = get_operand_type(builder, &rhs_tmp);

                        if matches!(lhs_type, Some(TypeNode::String))
                            || matches!(rhs_type, Some(TypeNode::String))
                        {
                            // Convert non-string operands to strings for concatenation
                            let lhs_for_concat = if matches!(lhs_type, Some(TypeNode::String)) {
                                lhs_tmp.clone()
                            } else {
                                // Cast non-string to string
                                let cast_tmp = builder.next_tmp();
                                let source_type = type_node_to_source_type(&lhs_type);
                                block.instrs.push(MirInstr::Cast {
                                    name: cast_tmp.clone(),
                                    value: lhs_tmp.clone(),
                                    source_type,
                                    target_type: "String".to_string(),
                                });
                                builder
                                    .mir_symbol_table
                                    .insert(cast_tmp.clone(), TypeNode::String);
                                cast_tmp
                            };

                            let rhs_for_concat = if matches!(rhs_type, Some(TypeNode::String)) {
                                rhs_tmp.clone()
                            } else {
                                // Cast non-string to string
                                let cast_tmp = builder.next_tmp();
                                let source_type = type_node_to_source_type(&rhs_type);
                                block.instrs.push(MirInstr::Cast {
                                    name: cast_tmp.clone(),
                                    value: rhs_tmp.clone(),
                                    source_type,
                                    target_type: "String".to_string(),
                                });
                                builder
                                    .mir_symbol_table
                                    .insert(cast_tmp.clone(), TypeNode::String);
                                cast_tmp
                            };

                            block.instrs.push(MirInstr::StringConcat {
                                name: dest_tmp.clone(),
                                left: lhs_for_concat,
                                right: rhs_for_concat,
                            });
                            builder
                                .mir_symbol_table
                                .insert(dest_tmp.clone(), TypeNode::String);
                        } else {
                            // Numeric addition - determine operation type
                            match determine_op_type(builder, &lhs_tmp, &rhs_tmp) {
                                Ok(op_type) if op_type == "string" => {
                                    // Convert non-string operands to strings for concatenation
                                    let lhs_type_inner = get_operand_type(builder, &lhs_tmp);
                                    let lhs_for_concat =
                                        if matches!(lhs_type_inner, Some(TypeNode::String)) {
                                            lhs_tmp.clone()
                                        } else {
                                            // Cast non-string to string
                                            let cast_tmp = builder.next_tmp();
                                            let source_type =
                                                type_node_to_source_type(&lhs_type_inner);
                                            block.instrs.push(MirInstr::Cast {
                                                name: cast_tmp.clone(),
                                                value: lhs_tmp.clone(),
                                                source_type,
                                                target_type: "String".to_string(),
                                            });
                                            builder
                                                .mir_symbol_table
                                                .insert(cast_tmp.clone(), TypeNode::String);
                                            cast_tmp
                                        };

                                    let rhs_type_inner = get_operand_type(builder, &rhs_tmp);
                                    let rhs_for_concat =
                                        if matches!(rhs_type_inner, Some(TypeNode::String)) {
                                            rhs_tmp.clone()
                                        } else {
                                            // Cast non-string to string
                                            let cast_tmp = builder.next_tmp();
                                            let source_type =
                                                type_node_to_source_type(&rhs_type_inner);
                                            block.instrs.push(MirInstr::Cast {
                                                name: cast_tmp.clone(),
                                                value: rhs_tmp.clone(),
                                                source_type,
                                                target_type: "String".to_string(),
                                            });
                                            builder
                                                .mir_symbol_table
                                                .insert(cast_tmp.clone(), TypeNode::String);
                                            cast_tmp
                                        };

                                    block.instrs.push(MirInstr::StringConcat {
                                        name: dest_tmp.clone(),
                                        left: lhs_for_concat,
                                        right: rhs_for_concat,
                                    });
                                    builder
                                        .mir_symbol_table
                                        .insert(dest_tmp.clone(), TypeNode::String);
                                }
                                Ok(op_type) => {
                                    block.instrs.push(MirInstr::BinaryOp(
                                        format!("add:{}", op_type),
                                        dest_tmp.clone(),
                                        lhs_tmp,
                                        rhs_tmp,
                                    ));
                                    // Track result type
                                    if op_type == "float" {
                                        builder
                                            .mir_symbol_table
                                            .insert(dest_tmp.clone(), TypeNode::Float);
                                    } else {
                                        builder
                                            .mir_symbol_table
                                            .insert(dest_tmp.clone(), TypeNode::Int);
                                    }
                                }
                                Err(err) => {
                                    debug_assert!(
                                        false,
                                        "Type error in addition: {} - should be caught by analyzer",
                                        err
                                    );
                                    // Continue with placeholder - analyzer should catch this
                                    block.instrs.push(MirInstr::BinaryOp(
                                        "add:int".to_string(),
                                        dest_tmp.clone(),
                                        lhs_tmp,
                                        rhs_tmp,
                                    ));
                                    builder
                                        .mir_symbol_table
                                        .insert(dest_tmp.clone(), TypeNode::Int);
                                }
                            }
                        }
                    } else {
                        // Other binary operators (sub, mul, div, comparisons, logical, etc.).
                        let op_str = match op {
                            TokenType::Minus => "sub",
                            TokenType::Star => "mul",
                            TokenType::Slash => "div",
                            TokenType::Gt => "gt",
                            TokenType::Lt => "lt",
                            TokenType::GtEq => "ge",
                            TokenType::LtEq => "le",
                            TokenType::EqEq => "eq",
                            TokenType::NotEq => "ne",
                            TokenType::Percent => "mod",
                            TokenType::AndAnd => "and",
                            TokenType::OrOr => "or",
                            _ => "unknown",
                        }
                        .to_string();

                        // Determine operation type based on operands
                        match determine_op_type(builder, &lhs_tmp, &rhs_tmp) {
                            Ok(op_type) if op_type == "string" => {
                                // String comparisons are only allowed for eq and ne
                                if op_str == "eq" || op_str == "ne" {
                                    block.instrs.push(MirInstr::BinaryOp(
                                        format!("{}:string", op_str),
                                        dest_tmp.clone(),
                                        lhs_tmp.clone(),
                                        rhs_tmp.clone(),
                                    ));
                                    builder
                                        .mir_symbol_table
                                        .insert(dest_tmp.clone(), TypeNode::Bool);
                                } else {
                                    debug_assert!(false, "Cannot perform '{}' operation on string types (only eq/ne allowed) - should be caught by analyzer", op_str);
                                    // Fallback: generate placeholder instruction
                                    block.instrs.push(MirInstr::BinaryOp(
                                        format!("{}:int", op_str),
                                        dest_tmp.clone(),
                                        lhs_tmp.clone(),
                                        rhs_tmp.clone(),
                                    ));
                                    builder
                                        .mir_symbol_table
                                        .insert(dest_tmp.clone(), TypeNode::Int);
                                }
                            }
                            Ok(op_type) => {
                                block.instrs.push(MirInstr::BinaryOp(
                                    format!("{}:{}", op_str, op_type),
                                    dest_tmp.clone(),
                                    lhs_tmp,
                                    rhs_tmp,
                                ));
                                // Track result type - comparisons and logical ops return bool, others return the operand type
                                if matches!(
                                    op_str.as_str(),
                                    "eq" | "ne" | "lt" | "le" | "gt" | "ge" | "and" | "or"
                                ) {
                                    builder
                                        .mir_symbol_table
                                        .insert(dest_tmp.clone(), TypeNode::Bool);
                                } else if op_type == "float" {
                                    builder
                                        .mir_symbol_table
                                        .insert(dest_tmp.clone(), TypeNode::Float);
                                } else {
                                    builder
                                        .mir_symbol_table
                                        .insert(dest_tmp.clone(), TypeNode::Int);
                                }
                            }
                            Err(err) => {
                                debug_assert!(false, "Type error in '{}' operation: {} - should be caught by analyzer", op_str, err);
                                // Fallback: generate placeholder instruction
                                block.instrs.push(MirInstr::BinaryOp(
                                    format!("{}:int", op_str),
                                    dest_tmp.clone(),
                                    lhs_tmp,
                                    rhs_tmp,
                                ));
                                builder
                                    .mir_symbol_table
                                    .insert(dest_tmp.clone(), TypeNode::Int);
                            }
                        }
                    }

                    dest_tmp
                }
            }
        }

        AstNode::FunctionCall { func, args } => {
            let func_name = match &**func {
                AstNode::Identifier(name) => name.clone(),
                _ => {
                    // If func is an expression, evaluate it and use its result as the function name.
                    build_expression(builder, func, block)
                }
            };

            // Check if this is actually a bare enum variant call (e.g., Success(42) instead of Result::Success(42))
            // Search through all enums to find if this name is a variant
            let mut found_enum: Option<(String, String)> = None;
            for (enum_name, variants) in builder.enum_table.iter() {
                for (variant_name, _) in variants {
                    if variant_name == &func_name {
                        found_enum = Some((enum_name.clone(), variant_name.clone()));
                        break;
                    }
                }
                if found_enum.is_some() {
                    break;
                }
            }

            if let Some((enum_name, variant_name)) = found_enum {
                // This is a bare enum variant call - convert to EnumInit
                let payload_tmp = if !args.is_empty() {
                    // Enum variants only support single payload
                    Some(build_expression(builder, &args[0], block))
                } else {
                    None
                };

                let enum_tmp = builder.next_tmp();
                let variant_index = get_enum_variant_index(builder, &enum_name, &variant_name);
                block.instrs.push(MirInstr::EnumInit {
                    name: enum_tmp.clone(),
                    enum_name: enum_name.clone(),
                    variant: variant_name.clone(),
                    variant_index,
                    value: payload_tmp,
                });

                // Track enum type in symbol table so Cast knows the source type
                if let Some(variants) = builder.enum_table.get(&enum_name) {
                    builder.mir_symbol_table.insert(
                        enum_tmp.clone(),
                        TypeNode::Enum(enum_name.clone(), variants.clone()),
                    );
                }

                enum_tmp
            } else {
                // Regular function call
                let mut arg_tmps = vec![];
                for arg in args {
                    let arg_tmp = build_expression(builder, arg, block);
                    arg_tmps.push(arg_tmp);
                }

                let dest_tmp = builder.next_tmp();
                block.instrs.push(MirInstr::Call {
                    dest: vec![dest_tmp.clone()],
                    func: func_name.clone(),
                    args: arg_tmps,
                });

                // CRITICAL: Track function return type in mir_symbol_table
                // This ensures binary operations with function call results use correct types
                if let Some((_params, return_type, _error_type)) =
                    builder.function_table.get(&func_name)
                {
                    builder
                        .mir_symbol_table
                        .insert(dest_tmp.clone(), return_type.clone());
                }

                dest_tmp
            }
        }

        AstNode::MethodCall {
            object,
            method,
            args,
        } => {
            // Check if object is a field access - we'll need to write back after mutating methods
            let is_field_access = matches!(object.as_ref(), AstNode::FieldAccess { .. });
            let (struct_instance, field_name) = if let AstNode::FieldAccess {
                object: struct_obj,
                field,
            } = object.as_ref()
            {
                // Get the struct instance temp
                let struct_tmp = build_expression(builder, struct_obj, block);
                (Some(struct_tmp), Some(field.clone()))
            } else {
                (None, None)
            };

            let object_tmp = build_expression(builder, object, block);
            let mut arg_tmps = vec![];
            for arg in args {
                let arg_tmp = build_expression(builder, arg, block);
                arg_tmps.push(arg_tmp);
            }

            let dest_tmp = builder.next_tmp();
            block.instrs.push(MirInstr::MethodCall {
                dest: dest_tmp.clone(),
                object: object_tmp.clone(),
                method: method.clone(),
                args: arg_tmps,
            });

            // CRITICAL: Track method return type in mir_symbol_table
            // This ensures the dest_tmp is properly typed for subsequent operations
            if let Some(obj_type) = builder.mir_symbol_table.get(&object_tmp).cloned() {
                // Get the type name for method_table lookup
                let type_name = match &obj_type {
                    TypeNode::TypeRef(name) => name.clone(),
                    TypeNode::Struct(name, _) => name.clone(),
                    TypeNode::Array(inner) => format!("Array({})", inner.format_type_string()),
                    TypeNode::Map(key, val) => format!(
                        "Map({},{})",
                        key.format_type_string(),
                        val.format_type_string()
                    ),
                    _ => String::new(),
                };

                // Look up method return type in method_table
                if !type_name.is_empty() {
                    if let Some(methods) = builder.method_table.get(&type_name) {
                        if let Some((_params, return_type, _error_type)) = methods.get(method) {
                            builder
                                .mir_symbol_table
                                .insert(dest_tmp.clone(), return_type.clone());
                        }
                    }
                }
            }

            // For mutating methods on struct fields, write the value back
            // This is critical for methods like push/pop that reallocate the underlying array
            if is_field_access && is_mutating_method(method) {
                if let (Some(struct_tmp), Some(field)) = (struct_instance, field_name) {
                    // Write the potentially modified value back to the struct field
                    block.instrs.push(MirInstr::StructSet {
                        struct_instance: struct_tmp,
                        field,
                        value: object_tmp,
                    });
                }
            }

            dest_tmp
        }

        AstNode::Closure {
            params,
            body,
            return_type,
            error_type: _, // TODO: Handle error_type for Result-returning closures
        } => {
            let closure_name = builder.next_tmp();

            // Extract parameter names and types
            let param_names: Vec<String> = params.iter().map(|(name, _)| name.clone()).collect();
            let param_types: Vec<Option<String>> = params
                .iter()
                .map(|(_, ty)| ty.as_ref().map(|t| format!("{:?}", t)))
                .collect();

            // DON'T evaluate closure body yet - store the AST for later codegen
            // This allows proper parameter binding when the closure is executed
            let body_expr = format!("closure_body_{}", closure_name);

            let return_type_str = return_type.as_ref().map(|t| format!("{:?}", t));

            // Create the closure instruction - store the AST body
            block.instrs.push(MirInstr::Closure {
                name: closure_name.clone(),
                params: param_names,
                param_types,
                body_expr,
                body_ast: Some(body.clone()), // Store the AST node
                return_type: return_type_str,
                captures: vec![],
            });

            // Track closure type in symbol table
            let param_type_nodes: Vec<TypeNode> = params
                .iter()
                .map(|(_, ty)| ty.clone().unwrap_or(TypeNode::Int))
                .collect();
            let return_type_node = return_type.clone().unwrap_or(TypeNode::Void);
            builder.mir_symbol_table.insert(
                closure_name.clone(),
                TypeNode::Function(param_type_nodes, Box::new(return_type_node)),
            );

            closure_name
        }

        AstNode::ArrayLiteral(elements) => {
            let mut tmp_elements = vec![];
            let mut element_type = TypeNode::Int; // Default element type

            for elem in elements {
                match elem {
                    AstNode::SpreadElement(inner) => {
                        // Handle spread operator: mark element as spread for codegen
                        let inner_tmp = build_expression(builder, inner, block);

                        // Get the array type to determine element type
                        if let Some(TypeNode::Array(elem_t)) = get_operand_type(builder, &inner_tmp)
                        {
                            if tmp_elements.is_empty() {
                                element_type = *elem_t;
                            }
                        }

                        // Add special marker for spread - codegen will handle expansion
                        tmp_elements.push(format!("SPREAD:{}", inner_tmp));
                    }
                    _ => {
                        let elem_tmp = build_expression(builder, elem, block);
                        // Track the type of the first element to use for the array
                        if tmp_elements.is_empty() {
                            if let Some(elem_t) = get_operand_type(builder, &elem_tmp) {
                                element_type = elem_t;
                            }
                        }
                        tmp_elements.push(elem_tmp);
                    }
                }
            }

            let tmp = builder.next_tmp();
            // Convert TypeNode to string for codegen
            let element_type_str = match &element_type {
                TypeNode::Int => Some("Int".to_string()),
                TypeNode::Float => Some("Float".to_string()),
                TypeNode::Bool => Some("Bool".to_string()),
                TypeNode::String => Some("Str".to_string()),
                TypeNode::TypeRef(struct_name) => Some(struct_name.clone()),
                _ => None,
            };
            block.instrs.push(MirInstr::Array {
                name: tmp.clone(),
                elements: tmp_elements,
                element_type: element_type_str,
            });
            // Track type in symbol table with proper element type
            builder
                .mir_symbol_table
                .insert(tmp.clone(), TypeNode::Array(Box::new(element_type)));
            tmp
        }

        AstNode::MapLiteral(entries) => {
            let mut map_entries = vec![];
            let mut key_type = TypeNode::String; // Default key type
            let mut value_type = TypeNode::Int; // Default value type

            for (key_expr, val_expr) in entries {
                match key_expr {
                    AstNode::SpreadElement(inner) => {
                        // Handle spread operator for maps: merge all entries from inner map
                        let inner_tmp = build_expression(builder, inner, block);

                        // Get the map type to determine key/value types
                        if let Some(TypeNode::Map(kt, vt)) = get_operand_type(builder, &inner_tmp) {
                            if map_entries.is_empty() {
                                key_type = *kt;
                                value_type = *vt;
                            }

                            // For now, we'll add a special marker that codegen needs to handle
                            // This is a simplified approach - the spread will be expanded at codegen
                            map_entries.push((inner_tmp, "SPREAD_MARKER".to_string()));
                        }
                    }
                    _ => {
                        let key_tmp = build_expression(builder, key_expr, block);
                        let val_tmp = build_expression(builder, val_expr, block);
                        // Track types from first entry
                        if map_entries.is_empty() {
                            if let Some(k_t) = get_operand_type(builder, &key_tmp) {
                                key_type = k_t;
                            }
                            if let Some(v_t) = get_operand_type(builder, &val_tmp) {
                                value_type = v_t;
                            }
                        }
                        map_entries.push((key_tmp, val_tmp));
                    }
                }
            }

            let tmp = builder.next_tmp();

            // Get string representation of types
            let key_type_str = match &key_type {
                TypeNode::Int => Some("Int".to_string()),
                TypeNode::Float => Some("Float".to_string()),
                TypeNode::Bool => Some("Bool".to_string()),
                TypeNode::String => Some("Str".to_string()),
                TypeNode::Struct(name, _) => Some(name.clone()),
                TypeNode::TypeRef(name) => Some(name.clone()),
                _ => None,
            };

            let value_type_str = match &value_type {
                TypeNode::Int => Some("Int".to_string()),
                TypeNode::Float => Some("Float".to_string()),
                TypeNode::Bool => Some("Bool".to_string()),
                TypeNode::String => Some("Str".to_string()),
                TypeNode::Struct(name, _) => Some(name.clone()),
                TypeNode::TypeRef(name) => Some(name.clone()),
                _ => None,
            };

            block.instrs.push(MirInstr::Map {
                name: tmp.clone(),
                entries: map_entries,
                key_type: key_type_str,
                value_type: value_type_str,
            });
            // Track type in symbol table with actual key and value types
            let map_type = TypeNode::Map(Box::new(key_type), Box::new(value_type));
            builder.mir_symbol_table.insert(tmp.clone(), map_type);
            tmp
        }

        // Element access: arr[index] or map[key] or arr[start..end] (slicing)
        AstNode::ElementAccess { array, index } => {
            let array_tmp = build_expression(builder, array, block);

            // Check if index is a range (for slicing)
            match index.as_ref() {
                AstNode::Range {
                    start,
                    end,
                    inclusive,
                } => {
                    // Array/string slicing: arr[start..end] or arr[start..=end]
                    let start_tmp = build_expression(builder, start, block);
                    let end_tmp = build_expression(builder, end, block);

                    let result_tmp = builder.next_tmp();

                    // Track the type - slicing returns the same array/string type
                    let arr_type = get_operand_type(builder, &array_tmp);

                    block.instrs.push(MirInstr::ArraySlice {
                        name: result_tmp.clone(),
                        array: array_tmp,
                        start: start_tmp,
                        end: end_tmp,
                        inclusive: *inclusive,
                    });

                    if let Some(arr_type) = arr_type {
                        builder
                            .mir_symbol_table
                            .insert(result_tmp.clone(), arr_type);
                    }

                    return result_tmp;
                }
                _ => {}
            }

            let index_tmp = build_expression(builder, index, block);

            // Check if it's an array or map access by looking up the type
            let array_type = get_operand_type(builder, &array_tmp);

            match array_type {
                // Array element access
                Some(TypeNode::Array(_)) => {
                    let result_tmp = builder.next_tmp();
                    block.instrs.push(MirInstr::ArrayGet {
                        name: result_tmp.clone(),
                        array: array_tmp,
                        index: index_tmp,
                    });
                    result_tmp
                }
                // Map element access
                Some(TypeNode::Map(_, value_type)) => {
                    let result_tmp = builder.next_tmp();
                    block.instrs.push(MirInstr::MapGet {
                        name: result_tmp.clone(),
                        map: array_tmp,
                        key: index_tmp,
                    });
                    // Track the value type
                    builder
                        .mir_symbol_table
                        .insert(result_tmp.clone(), *value_type);
                    result_tmp
                }
                // Fallback: treat as array access
                _ => {
                    let result_tmp = builder.next_tmp();
                    block.instrs.push(MirInstr::ArrayGet {
                        name: result_tmp.clone(),
                        array: array_tmp,
                        index: index_tmp,
                    });
                    result_tmp
                }
            }
        }

        // Type casting: expr as TargetType
        AstNode::Cast { expr, target_type } => {
            let value_tmp = build_expression(builder, expr, block);
            let result_tmp = builder.next_tmp();

            let target_type_str = match target_type {
                crate::parser::ast::TypeNode::Int => "Int".to_string(),
                crate::parser::ast::TypeNode::Float => "Float".to_string(),
                crate::parser::ast::TypeNode::String => "String".to_string(),
                crate::parser::ast::TypeNode::Bool => "Bool".to_string(),
                _ => "Int".to_string(),
            };

            // Determine source type
            let source_type_str = if let Some(source_type) = get_operand_type(builder, &value_tmp) {
                match source_type {
                    TypeNode::Int => "Int".to_string(),
                    TypeNode::Float => "Float".to_string(),
                    TypeNode::String => "String".to_string(),
                    TypeNode::Bool => "Bool".to_string(),
                    _ => "Int".to_string(),
                }
            } else {
                "Int".to_string()
            };

            block.instrs.push(MirInstr::Cast {
                name: result_tmp.clone(),
                value: value_tmp.clone(),
                source_type: source_type_str.clone(),
                target_type: target_type_str.clone(),
            });

            // Track result type in symbol table
            let result_type = match target_type {
                crate::parser::ast::TypeNode::Int => TypeNode::Int,
                crate::parser::ast::TypeNode::Float => TypeNode::Float,
                crate::parser::ast::TypeNode::String => TypeNode::String,
                crate::parser::ast::TypeNode::Bool => TypeNode::Bool,
                _ => TypeNode::Int,
            };
            builder
                .mir_symbol_table
                .insert(result_tmp.clone(), result_type);

            result_tmp
        }

        // Try propagate (? operator): expr?
        AstNode::TryPropagate { expr } => {
            // Build the expression that might fail
            let result_tmp = build_expression(builder, expr, block);

            // Create a temporary for the unwrapped value
            let unwrapped_tmp = builder.next_tmp();

            // Generate TryPropagate instruction
            // This will check the Result tag at runtime:
            // - If Err: return the Err immediately
            // - If Ok: extract and continue with the Ok value
            block.instrs.push(MirInstr::TryPropagate {
                name: unwrapped_tmp.clone(),
                result: result_tmp.clone(),
                error_block: String::new(), // Will be handled by codegen
            });

            // The unwrapped value has the Ok type of the Result
            // Copy type info from the result if available
            if let Some(result_type) = builder.mir_symbol_table.get(&result_tmp).cloned() {
                // If the result is a Result type, we need to extract the Ok type
                // For now, just copy the type - codegen will handle the unwrapping
                builder
                    .mir_symbol_table
                    .insert(unwrapped_tmp.clone(), result_type);
            }

            unwrapped_tmp
        }

        // UnwrapOrPanic (?? operator): expr ?? panic("message")
        AstNode::UnwrapOrPanic { expr, panic_msg } => {
            // Build the expression that might fail
            let result_tmp = build_expression(builder, expr, block);

            // Extract the panic message from the fallback expression
            // The fallback is typically panic("message"), so we need to extract the message argument
            let panic_msg_tmp = match panic_msg.as_ref() {
                // If it's a function call to panic, extract the first argument
                AstNode::FunctionCall { func, args } => {
                    if let AstNode::Identifier(name) = func.as_ref() {
                        if name == "panic" && !args.is_empty() {
                            // Build the message argument
                            build_expression(builder, &args[0], block)
                        } else {
                            // Not a panic call - build the whole expression
                            build_expression(builder, panic_msg, block)
                        }
                    } else {
                        build_expression(builder, panic_msg, block)
                    }
                }
                // If it's a string literal directly
                AstNode::StringLiteral(s) => {
                    let tmp = builder.next_tmp();
                    block.instrs.push(MirInstr::Assign {
                        name: tmp.clone(),
                        value: format!("\"{}\"", s),
                        mutable: false,
                    });
                    tmp
                }
                // Otherwise build whatever expression was provided
                _ => build_expression(builder, panic_msg, block),
            };

            // Create a temporary for the unwrapped value
            let unwrapped_tmp = builder.next_tmp();

            // Generate UnwrapOrPanic instruction
            // This will check the Result tag at runtime:
            // - If Err: panic with the provided message
            // - If Ok: extract and continue with the Ok value
            block.instrs.push(MirInstr::UnwrapOrPanic {
                name: unwrapped_tmp.clone(),
                result: result_tmp.clone(),
                panic_msg: panic_msg_tmp.clone(),
            });

            // The unwrapped value has the Ok type of the Result
            // Copy type info from the result if available
            if let Some(result_type) = builder.mir_symbol_table.get(&result_tmp).cloned() {
                builder
                    .mir_symbol_table
                    .insert(unwrapped_tmp.clone(), result_type);
            }

            unwrapped_tmp
        }

        // Ok expression as expression (e.g., inside return Ok x)
        AstNode::OkExpr { values } => {
            // Build MIR for each value expression
            let value_tmps: Vec<String> = values
                .iter()
                .map(|v| build_expression(builder, v, block))
                .collect();

            // Check if current function has error type
            // If no error type, Ok is just a value (not wrapped in Result struct)
            if builder.current_function_error_type.is_some() {
                // Function has error type - create a Result Ok instruction
                let result_tmp = builder.next_tmp();
                block.instrs.push(MirInstr::ResultOk {
                    name: result_tmp.clone(),
                    values: value_tmps,
                });
                result_tmp
            } else {
                // Function has no error type - Ok is just the value itself
                // Return the first value (or create empty temp if no values)
                if value_tmps.len() == 1 {
                    value_tmps[0].clone()
                } else if value_tmps.is_empty() {
                    // No values - return empty temp
                    let tmp = builder.next_tmp();
                    block.instrs.push(MirInstr::Assign {
                        name: tmp.clone(),
                        value: "0".to_string(),
                        mutable: false,
                    });
                    tmp
                } else {
                    // Multiple values - create tuple
                    let tuple_tmp = builder.next_tmp();
                    block.instrs.push(MirInstr::TupleCreate {
                        name: tuple_tmp.clone(),
                        elements: value_tmps,
                    });
                    tuple_tmp
                }
            }
        }

        // Err expression as expression (e.g., inside return Err msg)
        AstNode::ErrExpr { value } => {
            // Build MIR for the error value
            let error_tmp = build_expression(builder, value, block);

            // Create a Result Err instruction
            let result_tmp = builder.next_tmp();
            block.instrs.push(MirInstr::ResultErr {
                name: result_tmp.clone(),
                error: error_tmp,
            });

            result_tmp
        }

        // Struct literal: Point { x: 10, y: 20 }
        AstNode::StructLiteral { name, fields } => {
            // Build MIR for each field value expression
            let mut field_values = Vec::new();

            // Get struct field types from the struct table to handle empty arrays correctly
            let struct_field_types = builder.program.struct_table.get(name).cloned();

            for (field_name, field_expr) in fields {
                // For empty array literals, check if we know the expected field type
                if let AstNode::ArrayLiteral(elements) = field_expr.as_ref() {
                    if elements.is_empty() {
                        if let Some(ref field_types) = struct_field_types {
                            if let Some(TypeNode::Array(elem_type)) = field_types.get(field_name) {
                                // Build empty array with known element type
                                let tmp = builder.next_tmp();
                                let element_type_str = match elem_type.as_ref() {
                                    TypeNode::Int => Some("Int".to_string()),
                                    TypeNode::Float => Some("Float".to_string()),
                                    TypeNode::Bool => Some("Bool".to_string()),
                                    TypeNode::String => Some("Str".to_string()),
                                    TypeNode::TypeRef(struct_name) => Some(struct_name.clone()),
                                    TypeNode::Struct(struct_name, _) => Some(struct_name.clone()),
                                    _ => None,
                                };
                                block.instrs.push(MirInstr::Array {
                                    name: tmp.clone(),
                                    elements: vec![],
                                    element_type: element_type_str,
                                });
                                builder
                                    .mir_symbol_table
                                    .insert(tmp.clone(), TypeNode::Array(elem_type.clone()));
                                field_values.push((field_name.clone(), tmp));
                                continue;
                            }
                        }
                    }
                }

                // For empty map literals, check if we know the expected field type
                if let AstNode::MapLiteral(entries) = field_expr.as_ref() {
                    if entries.is_empty() {
                        if let Some(ref field_types) = struct_field_types {
                            if let Some(TypeNode::Map(key_type, value_type)) =
                                field_types.get(field_name)
                            {
                                // Build empty map with known key/value types
                                let tmp = builder.next_tmp();
                                let key_type_str = match key_type.as_ref() {
                                    TypeNode::Int => Some("Int".to_string()),
                                    TypeNode::Float => Some("Float".to_string()),
                                    TypeNode::Bool => Some("Bool".to_string()),
                                    TypeNode::String => Some("Str".to_string()),
                                    TypeNode::TypeRef(struct_name) => Some(struct_name.clone()),
                                    TypeNode::Struct(struct_name, _) => Some(struct_name.clone()),
                                    _ => None,
                                };
                                let value_type_str = match value_type.as_ref() {
                                    TypeNode::Int => Some("Int".to_string()),
                                    TypeNode::Float => Some("Float".to_string()),
                                    TypeNode::Bool => Some("Bool".to_string()),
                                    TypeNode::String => Some("Str".to_string()),
                                    TypeNode::TypeRef(struct_name) => Some(struct_name.clone()),
                                    TypeNode::Struct(struct_name, _) => Some(struct_name.clone()),
                                    _ => None,
                                };
                                block.instrs.push(MirInstr::Map {
                                    name: tmp.clone(),
                                    entries: vec![],
                                    key_type: key_type_str,
                                    value_type: value_type_str,
                                });
                                builder.mir_symbol_table.insert(
                                    tmp.clone(),
                                    TypeNode::Map(key_type.clone(), value_type.clone()),
                                );
                                field_values.push((field_name.clone(), tmp));
                                continue;
                            }
                        }
                    }
                }

                let value_tmp = build_expression(builder, field_expr, block);
                field_values.push((field_name.clone(), value_tmp));
            }

            // Create struct initialization instruction
            let struct_tmp = builder.next_tmp();
            block.instrs.push(MirInstr::StructInit {
                name: struct_tmp.clone(),
                struct_name: name.clone(),
                fields: field_values,
            });

            // Track struct type in symbol table so Cast knows the source type
            builder
                .mir_symbol_table
                .insert(struct_tmp.clone(), TypeNode::TypeRef(name.clone()));

            struct_tmp
        }

        // Field access: obj.field
        AstNode::FieldAccess { object, field } => {
            // Build MIR for the object expression
            let object_tmp = build_expression(builder, object, block);

            // Create field access instruction
            let field_tmp = builder.next_tmp();
            block.instrs.push(MirInstr::StructGet {
                name: field_tmp.clone(),
                struct_instance: object_tmp,
                field: field.clone(),
            });

            field_tmp
        }

        // Enum variant: Direction::North or Status::Active(value)
        // OR namespaced function call: File::Read(...) - distinguish at MIR generation time
        AstNode::EnumVariant {
            enum_name,
            variant,
            payload,
        } => {
            // Check if this is actually an enum variant or a namespaced function call
            let is_enum = builder.enum_table.contains_key(enum_name);
            let qualified_name = format!("{}::{}", enum_name, variant);
            let is_function = builder.function_table.contains_key(&qualified_name);

            if is_enum {
                // It's an actual enum variant
                // Build MIR for payload if present
                let payload_tmp = if !payload.is_empty() {
                    if payload.len() == 1 {
                        // Single payload - build directly
                        Some(build_expression(builder, &payload[0], block))
                    } else {
                        // Multiple payloads - create a tuple
                        let mut element_tmps = vec![];
                        for elem in payload {
                            let elem_tmp = build_expression(builder, elem, block);
                            element_tmps.push(elem_tmp);
                        }

                        // Create tuple from the elements
                        let tuple_tmp = builder.next_tmp();
                        block.instrs.push(MirInstr::TupleCreate {
                            name: tuple_tmp.clone(),
                            elements: element_tmps,
                        });

                        Some(tuple_tmp)
                    }
                } else {
                    None
                };

                // Create enum initialization instruction
                let enum_tmp = builder.next_tmp();
                let variant_index = get_enum_variant_index(builder, enum_name, variant);
                block.instrs.push(MirInstr::EnumInit {
                    name: enum_tmp.clone(),
                    enum_name: enum_name.clone(),
                    variant: variant.clone(),
                    variant_index,
                    value: payload_tmp,
                });

                // Track enum type in symbol table so Cast knows the source type
                if let Some(variants) = builder.enum_table.get(enum_name) {
                    builder.mir_symbol_table.insert(
                        enum_tmp.clone(),
                        TypeNode::Enum(enum_name.clone(), variants.clone()),
                    );
                }

                enum_tmp
            } else if is_function {
                // It's a namespaced function call (e.g., File::Read, File::Write)
                // Build MIR for all arguments
                let mut arg_tmps = vec![];
                for arg in payload {
                    let arg_tmp = build_expression(builder, arg, block);
                    arg_tmps.push(arg_tmp);
                }

                let dest_tmp = builder.next_tmp();
                block.instrs.push(MirInstr::Call {
                    dest: vec![dest_tmp.clone()],
                    func: qualified_name,
                    args: arg_tmps,
                });

                dest_tmp
            } else {
                // Neither enum nor function - this should have been caught by analyzer
                // Generate a placeholder to avoid panic
                let error_tmp = builder.next_tmp();
                block.instrs.push(MirInstr::Assign {
                    name: error_tmp.clone(),
                    value: "nil".to_string(),
                    mutable: false,
                });
                error_tmp
            }
        }

        // Conditional expression (inline if-else)
        AstNode::ConditionalExpr {
            condition,
            then_expr,
            else_expr,
        } => {
            // Evaluate condition
            let cond_tmp = build_expression(builder, condition, block);

            // Create blocks for then and else branches
            let then_label = builder.next_block();
            let else_label = builder.next_block();
            let merge_label = builder.next_block();

            // Allocate result variable before branching
            let result_tmp = builder.next_tmp();

            // Set terminator for current block
            block.terminator = Some(MirInstr::CondJump {
                cond: cond_tmp,
                then_block: then_label.clone(),
                else_block: else_label.clone(),
            });

            // Save the current block with its terminator
            let original_block = MirBlock {
                label: block.label.clone(),
                instrs: block.instrs.clone(),
                terminator: block.terminator.clone(),
            };

            // Then block
            let mut then_block = MirBlock {
                label: then_label.clone(),
                instrs: vec![],
                terminator: None,
            };
            let then_result = build_expression(builder, then_expr, &mut then_block);
            then_block.instrs.push(MirInstr::Assign {
                name: result_tmp.clone(),
                value: then_result,
                mutable: false,
            });
            then_block.terminator = Some(MirInstr::Jump {
                label: merge_label.clone(),
            });

            // Else block
            let mut else_block = MirBlock {
                label: else_label.clone(),
                instrs: vec![],
                terminator: None,
            };
            let else_result = build_expression(builder, else_expr, &mut else_block);
            else_block.instrs.push(MirInstr::Assign {
                name: result_tmp.clone(),
                value: else_result,
                mutable: false,
            });
            else_block.terminator = Some(MirInstr::Jump {
                label: merge_label.clone(),
            });

            // Push all blocks to function
            if let Some(current_func) = builder.program.functions.last_mut() {
                current_func.blocks.push(original_block);
                current_func.blocks.push(then_block);
                current_func.blocks.push(else_block);
            }

            // Replace current block with merge label continuation
            block.label = merge_label;
            block.instrs.clear();
            block.terminator = None;

            // Return the result variable which both branches assign to
            result_tmp
        }

        // Ternary expression (condition ? true_expr : false_expr)
        AstNode::TernaryExpr {
            condition,
            true_expr,
            false_expr,
        } => {
            // Use ConditionalExpr (if-else expression) instead, which properly handles cross-block values
            let if_expr = AstNode::ConditionalExpr {
                condition: condition.clone(),
                then_expr: true_expr.clone(),
                else_expr: false_expr.clone(),
            };
            return build_expression(builder, &if_expr, block);
        }

        // Match expression
        AstNode::MatchExpr { values, arms } => {
            // Evaluate the match values if present
            let value_tmps: Vec<String> = values
                .iter()
                .map(|v| build_expression(builder, v, block))
                .collect();

            // For single-value match, use first value; for tuple match, we compare each element
            let value_tmp = value_tmps.first().cloned().unwrap_or_default();
            let is_tuple_match = value_tmps.len() > 1;

            // Create merge block that all arms will jump to
            let merge_label = builder.next_block();
            let result_tmp = builder.next_tmp();

            // Match expressions always need a result, even if arms are statement-like
            // We'll assign a unit value (0) for statement arms
            let all_arms_statements = false;

            // Pre-allocate all labels first
            let mut arm_labels = Vec::new();
            let mut check_labels = Vec::new();

            for i in 0..arms.len() {
                arm_labels.push(builder.next_block());
                if i > 0 {
                    check_labels.push(builder.next_block());
                }
            }

            // The current block becomes the first check block
            // We'll need to save it and create a new merge block at the end
            let first_check_label = block.label.clone();
            let first_check_instrs = block.instrs.clone();

            // Process each arm to generate check logic
            for (i, arm) in arms.iter().enumerate() {
                let arm_label = &arm_labels[i];
                let check_label = if i == 0 {
                    first_check_label.clone()
                } else {
                    check_labels[i - 1].clone()
                };

                // Determine next label for else branch
                let next_label = if i + 1 < arms.len() {
                    check_labels
                        .get(i)
                        .cloned()
                        .unwrap_or_else(|| merge_label.clone())
                } else {
                    // Last arm - if wildcard, always matches, else jump to merge
                    if matches!(arm.pattern, crate::parser::ast::MatchPattern::Wildcard) {
                        arm_label.clone()
                    } else {
                        merge_label.clone()
                    }
                };

                // Build check block for this arm
                let mut check_block = if i == 0 {
                    // First check - use the current block's instructions
                    MirBlock {
                        label: check_label.clone(),
                        instrs: first_check_instrs.clone(),
                        terminator: None,
                    }
                } else {
                    // Subsequent checks - create new block
                    MirBlock {
                        label: check_label.clone(),
                        instrs: vec![],
                        terminator: None,
                    }
                };

                match &arm.pattern {
                    crate::parser::ast::MatchPattern::Wildcard => {
                        // Wildcard always matches - unconditional jump
                        check_block.terminator = Some(MirInstr::Jump {
                            label: arm_label.clone(),
                        });
                    }
                    crate::parser::ast::MatchPattern::Literal(lit) => {
                        // Value-based match: compare value_tmp == literal
                        let lit_tmp = build_expression(builder, lit, &mut check_block);
                        let cond_tmp = builder.next_tmp();

                        // Determine the comparison type based on operands
                        let op_type = match determine_op_type(builder, &value_tmp, &lit_tmp) {
                            Ok(t) => t,
                            Err(_) => "int".to_string(),
                        };

                        check_block.instrs.push(MirInstr::BinaryOp(
                            format!("eq:{}", op_type),
                            cond_tmp.clone(),
                            value_tmp.clone(),
                            lit_tmp,
                        ));
                        check_block.terminator = Some(MirInstr::CondJump {
                            cond: cond_tmp,
                            then_block: arm_label.clone(),
                            else_block: next_label.clone(),
                        });
                    }
                    crate::parser::ast::MatchPattern::Tuple(patterns) => {
                        // Tuple match: compare each element
                        // Build comparisons for each pattern element
                        let mut all_match_tmp = String::new();

                        for (idx, pattern) in patterns.iter().enumerate() {
                            let val_tmp = if idx < value_tmps.len() {
                                value_tmps[idx].clone()
                            } else {
                                value_tmp.clone()
                            };

                            let elem_cond = match pattern {
                                crate::parser::ast::MatchPattern::Wildcard => {
                                    // Wildcard matches anything - always true
                                    let true_tmp = builder.next_tmp();
                                    check_block.instrs.push(MirInstr::ConstBool {
                                        name: true_tmp.clone(),
                                        value: true,
                                    });
                                    true_tmp
                                }
                                crate::parser::ast::MatchPattern::Literal(lit) => {
                                    let lit_tmp = build_expression(builder, lit, &mut check_block);
                                    let cmp_tmp = builder.next_tmp();
                                    let op_type =
                                        match determine_op_type(builder, &val_tmp, &lit_tmp) {
                                            Ok(t) => t,
                                            Err(_) => "int".to_string(),
                                        };
                                    check_block.instrs.push(MirInstr::BinaryOp(
                                        format!("eq:{}", op_type),
                                        cmp_tmp.clone(),
                                        val_tmp,
                                        lit_tmp,
                                    ));
                                    cmp_tmp
                                }
                                _ => {
                                    // For other pattern types in tuple, treat as always match for now
                                    let true_tmp = builder.next_tmp();
                                    check_block.instrs.push(MirInstr::ConstBool {
                                        name: true_tmp.clone(),
                                        value: true,
                                    });
                                    true_tmp
                                }
                            };

                            if all_match_tmp.is_empty() {
                                all_match_tmp = elem_cond;
                            } else {
                                // AND with previous result
                                let and_tmp = builder.next_tmp();
                                check_block.instrs.push(MirInstr::BinaryOp(
                                    "and:bool".to_string(),
                                    and_tmp.clone(),
                                    all_match_tmp,
                                    elem_cond,
                                ));
                                all_match_tmp = and_tmp;
                            }
                        }

                        check_block.terminator = Some(MirInstr::CondJump {
                            cond: all_match_tmp,
                            then_block: arm_label.clone(),
                            else_block: next_label.clone(),
                        });
                    }
                    crate::parser::ast::MatchPattern::Condition(cond) => {
                        // Condition-based match: evaluate condition
                        let cond_tmp = build_expression(builder, cond, &mut check_block);
                        check_block.terminator = Some(MirInstr::CondJump {
                            cond: cond_tmp,
                            then_block: arm_label.clone(),
                            else_block: next_label.clone(),
                        });
                    }
                    crate::parser::ast::MatchPattern::EnumVariant { enum_name, variant } => {
                        // Enum variant match without payload: compare enum tag
                        // Extract tag from the enum value
                        let value_tag_tmp = builder.next_tmp();
                        check_block.instrs.push(MirInstr::EnumGetTag {
                            name: value_tag_tmp.clone(),
                            enum_value: value_tmp.clone(),
                        });

                        // Create an enum variant temporary to compare against
                        let variant_tmp = builder.next_tmp();
                        let variant_index = get_enum_variant_index(builder, enum_name, variant);
                        check_block.instrs.push(MirInstr::EnumInit {
                            name: variant_tmp.clone(),
                            enum_name: enum_name.clone(),
                            variant: variant.clone(),
                            variant_index,
                            value: None,
                        });

                        // Extract tag from the variant
                        let variant_tag_tmp = builder.next_tmp();
                        check_block.instrs.push(MirInstr::EnumGetTag {
                            name: variant_tag_tmp.clone(),
                            enum_value: variant_tmp.clone(),
                        });

                        // Compare the tags
                        let cond_tmp = builder.next_tmp();
                        check_block.instrs.push(MirInstr::BinaryOp(
                            "eq:int".to_string(),
                            cond_tmp.clone(),
                            value_tag_tmp,
                            variant_tag_tmp,
                        ));
                        check_block.terminator = Some(MirInstr::CondJump {
                            cond: cond_tmp,
                            then_block: arm_label.clone(),
                            else_block: next_label.clone(),
                        });
                    }
                    crate::parser::ast::MatchPattern::EnumVariantWithPayload {
                        enum_name,
                        variant,
                        bindings,
                    } => {
                        // Enum variant match with payload: check tag and extract payload
                        // Extract tag from the enum value
                        let value_tag_tmp = builder.next_tmp();
                        check_block.instrs.push(MirInstr::EnumGetTag {
                            name: value_tag_tmp.clone(),
                            enum_value: value_tmp.clone(),
                        });

                        // Create an enum variant temporary to compare against
                        let variant_tmp = builder.next_tmp();
                        let variant_index = get_enum_variant_index(builder, enum_name, variant);
                        check_block.instrs.push(MirInstr::EnumInit {
                            name: variant_tmp.clone(),
                            enum_name: enum_name.clone(),
                            variant: variant.clone(),
                            variant_index,
                            value: None, // For comparison, we don't need the payload value
                        });

                        // Extract tag from the variant
                        let variant_tag_tmp = builder.next_tmp();
                        check_block.instrs.push(MirInstr::EnumGetTag {
                            name: variant_tag_tmp.clone(),
                            enum_value: variant_tmp.clone(),
                        });

                        // Compare the tags
                        let cond_tmp = builder.next_tmp();
                        check_block.instrs.push(MirInstr::BinaryOp(
                            "eq:int".to_string(),
                            cond_tmp.clone(),
                            value_tag_tmp,
                            variant_tag_tmp,
                        ));

                        // Look up the payload type from enum_table and store in symbol table
                        let payload_type = builder
                            .enum_table
                            .get(enum_name)
                            .and_then(|variants| variants.get(variant))
                            .and_then(|opt_type| opt_type.clone());

                        // Store bindings in symbol table
                        if let Some(ref ptype) = payload_type {
                            // For tuple types, we need to handle multiple bindings
                            if let TypeNode::Tuple(types) = ptype {
                                for (i, binding) in bindings.iter().enumerate() {
                                    let elem_type = types.get(i).cloned().unwrap_or(TypeNode::Int);
                                    builder.mir_symbol_table.insert(binding.clone(), elem_type);
                                }
                            } else {
                                // Single binding
                                if let Some(binding) = bindings.first() {
                                    builder
                                        .mir_symbol_table
                                        .insert(binding.clone(), ptype.clone());
                                }
                            }
                        } else {
                            // Fallback to Int if type lookup fails
                            for binding in bindings {
                                builder
                                    .mir_symbol_table
                                    .insert(binding.clone(), TypeNode::Int);
                            }
                        }

                        check_block.terminator = Some(MirInstr::CondJump {
                            cond: cond_tmp,
                            then_block: arm_label.clone(),
                            else_block: next_label.clone(),
                        });
                    }
                }

                // Immediately add check block to function
                if let Some(current_func) = builder.program.functions.last_mut() {
                    current_func.blocks.push(check_block);
                }
            }

            // Now build arm blocks
            for (i, arm) in arms.iter().enumerate() {
                let arm_label = &arm_labels[i];

                let mut arm_block = MirBlock {
                    label: arm_label.clone(),
                    instrs: vec![],
                    terminator: None,
                };

                // If this arm has a payload binding, extract it at the start of the arm block
                // Use the user's binding name directly - EnumGetPayload only stores to temp_values
                let _payload_binding_names =
                    if let crate::parser::ast::MatchPattern::EnumVariantWithPayload {
                        enum_name,
                        variant,
                        bindings,
                    } = &arm.pattern
                    {
                        // Look up the payload type from enum_table
                        let payload_type = builder
                            .enum_table
                            .get(enum_name)
                            .and_then(|variants| variants.get(variant))
                            .and_then(|opt_type| opt_type.clone());

                        // Handle multiple bindings for tuple payloads
                        if bindings.len() == 1 {
                            // Single binding - extract payload directly
                            arm_block.instrs.push(MirInstr::EnumGetPayload {
                                name: bindings[0].clone(),
                                enum_value: value_tmp.clone(),
                                enum_name: enum_name.clone(),
                                variant: variant.clone(),
                                payload_type: payload_type.clone(),
                            });

                            // Store the binding type in symbol table for type checking
                            if let Some(ref ptype) = payload_type {
                                builder
                                    .mir_symbol_table
                                    .insert(bindings[0].clone(), ptype.clone());
                            } else {
                                builder
                                    .mir_symbol_table
                                    .insert(bindings[0].clone(), TypeNode::Int);
                            }
                        } else {
                            // Multiple bindings - tuple payload
                            // First extract the tuple payload
                            let tuple_tmp = builder.next_tmp();
                            arm_block.instrs.push(MirInstr::EnumGetPayload {
                                name: tuple_tmp.clone(),
                                enum_value: value_tmp.clone(),
                                enum_name: enum_name.clone(),
                                variant: variant.clone(),
                                payload_type: payload_type.clone(),
                            });

                            // Get the tuple element types if available
                            let element_types = if let Some(TypeNode::Tuple(types)) = &payload_type
                            {
                                types.clone()
                            } else {
                                // Default to Int for all elements
                                vec![TypeNode::Int; bindings.len()]
                            };

                            // Extract each tuple element into its binding
                            for (i, binding) in bindings.iter().enumerate() {
                                arm_block.instrs.push(MirInstr::TupleGet {
                                    name: binding.clone(),
                                    tuple: tuple_tmp.clone(),
                                    index: i,
                                });

                                // Store the binding type
                                let elem_type =
                                    element_types.get(i).cloned().unwrap_or(TypeNode::Int);
                                builder.mir_symbol_table.insert(binding.clone(), elem_type);
                            }
                        }

                        Some(bindings.clone())
                    } else {
                        None
                    };

                // Check if arm body is a statement or expression
                match arm.body.as_ref() {
                    AstNode::Print { .. } => {
                        build_statement(builder, &arm.body, &mut arm_block);

                        // Always assign a unit/void value for statement arms
                        let unit_tmp = builder.next_tmp();
                        arm_block.instrs.push(MirInstr::ConstInt {
                            name: unit_tmp.clone(),
                            value: 0,
                        });
                        arm_block.instrs.push(MirInstr::Assign {
                            name: result_tmp.clone(),
                            value: unit_tmp,
                            mutable: false,
                        });
                    }
                    AstNode::Block(statements) => {
                        let mut last_result = String::new();
                        for (idx, stmt) in statements.iter().enumerate() {
                            if idx == statements.len() - 1 {
                                match stmt {
                                    AstNode::Print { .. }
                                    | AstNode::Break
                                    | AstNode::Continue
                                    | AstNode::Return { .. } => {
                                        build_statement(builder, stmt, &mut arm_block);
                                        last_result = String::new();
                                    }
                                    _ => {
                                        last_result =
                                            build_expression(builder, stmt, &mut arm_block);
                                    }
                                }
                            } else {
                                build_statement(builder, stmt, &mut arm_block);
                            }
                        }

                        // Always assign result for block arms
                        if last_result.is_empty() {
                            let unit_tmp = builder.next_tmp();
                            arm_block.instrs.push(MirInstr::ConstInt {
                                name: unit_tmp.clone(),
                                value: 0,
                            });
                            arm_block.instrs.push(MirInstr::Assign {
                                name: result_tmp.clone(),
                                value: unit_tmp,
                                mutable: false,
                            });
                        } else {
                            arm_block.instrs.push(MirInstr::Assign {
                                name: result_tmp.clone(),
                                value: last_result,
                                mutable: false,
                            });
                        }
                    }
                    AstNode::Break | AstNode::Continue | AstNode::Return { .. } => {
                        build_statement(builder, &arm.body, &mut arm_block);
                    }
                    AstNode::MatchExpr { .. } => {
                        // Special handling for nested MatchExpr:
                        // The nested match will take over the block and create its own control flow.
                        // We need to:
                        // 1. First commit the current arm_block with its payload extraction instructions
                        // 2. Create a new block for the nested match to use
                        // 3. Let the nested match build its own blocks
                        // 4. After nested match, assign result and jump to outer merge

                        // Save the payload extraction instructions that we've already added
                        let saved_instrs = arm_block.instrs.clone();
                        let saved_label = arm_block.label.clone();

                        // Create a fresh block for the nested match to use as its entry
                        let nested_entry_label = builder.next_block();
                        let mut nested_entry_block = MirBlock {
                            label: nested_entry_label.clone(),
                            instrs: vec![],
                            terminator: None,
                        };

                        // Build the nested match expression using the fresh block
                        let nested_result =
                            build_expression(builder, &arm.body, &mut nested_entry_block);

                        // The nested match will have transformed nested_entry_block into its merge block
                        // We need to add the result assignment and jump to outer merge
                        nested_entry_block.instrs.push(MirInstr::Assign {
                            name: result_tmp.clone(),
                            value: nested_result,
                            mutable: false,
                        });
                        nested_entry_block.terminator = Some(MirInstr::Jump {
                            label: merge_label.clone(),
                        });

                        // Restore the original arm_block with saved instructions
                        arm_block.label = saved_label;
                        arm_block.instrs = saved_instrs;
                        // Make the arm_block jump to the nested match entry
                        arm_block.terminator = Some(MirInstr::Jump {
                            label: nested_entry_label.clone(),
                        });

                        // Add the arm_block to function (it has payload extraction and jumps to nested match)
                        if let Some(current_func) = builder.program.functions.last_mut() {
                            current_func.blocks.push(arm_block.clone());
                        }

                        // Add the nested match's merge block (which is now nested_entry_block)
                        if let Some(current_func) = builder.program.functions.last_mut() {
                            current_func.blocks.push(nested_entry_block);
                        }

                        // Skip the normal arm_block handling below since we've already handled it
                        continue;
                    }
                    _ => {
                        let arm_result = build_expression(builder, &arm.body, &mut arm_block);

                        // Always assign result for expression arms
                        arm_block.instrs.push(MirInstr::Assign {
                            name: result_tmp.clone(),
                            value: arm_result,
                            mutable: false,
                        });
                    }
                }

                // Jump to merge
                arm_block.terminator = Some(MirInstr::Jump {
                    label: merge_label.clone(),
                });

                // Immediately add arm block to function
                if let Some(current_func) = builder.program.functions.last_mut() {
                    current_func.blocks.push(arm_block);
                }
            }

            // Now create a new merge block for continuation
            // The current block has been consumed as the first check block
            block.label = merge_label;
            block.instrs = vec![];
            block.terminator = None;

            result_tmp
        }

        // Block expression: { statements; result_expr }
        AstNode::BlockExpr { statements, result } => {
            // Build all statements first
            for stmt in statements {
                build_statement(builder, stmt, block);
            }
            // Then build and return the result expression
            build_expression(builder, result, block)
        }

        // Block (used as expression) - treat last item as result if it's an expression
        AstNode::Block(items) => {
            if items.is_empty() {
                // Empty block returns nil
                let tmp = builder.next_tmp();
                block.instrs.push(MirInstr::ConstInt {
                    name: tmp.clone(),
                    value: 0,
                });
                tmp
            } else {
                // Build all but the last as statements
                for stmt in items.iter().take(items.len() - 1) {
                    build_statement(builder, stmt, block);
                }
                // Last item is the result expression
                build_expression(builder, items.last().unwrap(), block)
            }
        }

        _ => {
            // For unhandled expressions, create a placeholder temporary.
            // This is a safeguard for future AST node types.
            builder.next_tmp()
        }
    };

    builder.recursion_depth -= 1;
    result
}
