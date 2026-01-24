//! MIR Builder
//!
//! Converts HIR to MIR with lowering of high-level constructs.

pub mod stmt;
pub mod expr;
pub mod pattern;

use doo_core::Span as CoreSpan;
use doo_core::types::{TypeId as CoreTypeId, TypeKind, TypeRegistry, builtin};
use doo_hir::{
    HirProgram, HirItem, HirFunction, HirStmt, 
    HirExpr, HirBinOp, HirUnaryOp, ConstValue, HirMatchPattern,
};

use rustc_hash::FxHashMap;

use crate::types::*;

/// HIR to MIR builder.
pub struct MirBuilder<'a> {
    /// Current function being built.
    pub(crate) current_func: Option<MirFunction>,
    /// Current block index.
    pub(crate) current_block: usize,
    /// Temporary counter for unique names.
    pub(crate) temp_counter: usize,
    /// Block counter for unique labels.
    pub(crate) block_counter: usize,

    pub(crate) type_registry: &'a TypeRegistry,
    pub(crate) container_kinds: FxHashMap<String, ContainerKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainerKind {
    Array,
    Map,
}

impl<'a> MirBuilder<'a> {
    /// Create a new MIR builder.
    pub fn new(type_registry: &'a TypeRegistry) -> Self {
        Self {
            current_func: None,
            current_block: 0,
            temp_counter: 0,
            block_counter: 0,
            type_registry,
            container_kinds: FxHashMap::default(),
        }
    }

    /// Build MIR from HIR program.
    pub fn build(&mut self, hir: &HirProgram) -> MirProgram {
        let mut program = MirProgram::new();

        for item in &hir.items {
            match item {
                HirItem::Function(f) => {
                    let mir_func = self.build_function(f);
                    program.functions.push(mir_func);
                }
                HirItem::Struct(s) => {
                    let mir_struct = StructDef {
                        name: s.name.clone(),
                        fields: s.fields.iter().map(|f| FieldDef {
                            name: f.name.clone(),
                            type_id: f.type_id.unwrap_or(builtin::ANY),
                            optional: f.is_optional,
                            decorators: Vec::new(),
                            default_value: None,
                        }).collect(),
                        decorators: Vec::new(),
                    };
                    program.structs.insert(s.name.clone(), mir_struct);
                }
                HirItem::Enum(e) => {
                    let mir_enum = EnumDef {
                        name: e.name.clone(),
                        variants: e.variants.iter().enumerate().map(|(i, v)| VariantDef {
                            name: v.name.clone(),
                            index: i as u32,
                            payload_type: v.payload,
                        }).collect(),
                    };
                    program.enums.insert(e.name.clone(), mir_enum);
                }
                HirItem::Import(_) => {
                    // Imports handled elsewhere
                }
            }
        }

        program
    }

    /// Build MIR for a function.
    fn build_function(&mut self, hir: &HirFunction) -> MirFunction {
        self.temp_counter = 0;
        self.block_counter = 0;
        self.container_kinds.clear();

        let mut func = MirFunction::new(hir.name.clone());
        func.params = hir.params.iter().map(|p| ParamDef {
            name: p.name.clone(),
            type_id: p.type_id.unwrap_or(builtin::ANY),
        }).collect();
        func.return_type = hir.return_type;
        func.error_type = hir.error_type;

        // Create entry block
        func.blocks.push(MirBlock::new("entry".to_string()));
        self.current_func = Some(func);
        self.current_block = 0;

        // Register parameters as locals
        for param in &hir.params {
            if let Some(f) = &mut self.current_func {
                f.locals.push(LocalDef {
                    name: param.name.clone(),
                    type_id: param.type_id.unwrap_or(builtin::ANY),
                    mutable: false,
                });
            }
        }

        // Build statements
        for stmt in &hir.body {
            self.build_stmt(stmt);
        }

        // Ensure function has a terminator
        if let Some(f) = &mut self.current_func {
            if let Some(block) = f.blocks.get_mut(self.current_block) {
                if matches!(block.terminator, MirTerminator::Unreachable) {
                    block.terminator = MirTerminator::Return { values: Vec::new() };
                }
            }
        }

        self.current_func.take().unwrap()
    }

    pub fn build_stmt(&mut self, stmt: &HirStmt) {
        stmt::build_stmt(self, stmt);
    }

    pub fn build_expr(&mut self, expr: &HirExpr) -> MirOperand {
        expr::build_expr(self, expr)
    }

    pub fn build_match_condition(
        &mut self,
        scrutinees: &[MirOperand],
        pattern: &HirMatchPattern,
        span: Span,
    ) -> MirOperand {
        pattern::build_match_condition(self, scrutinees, pattern, span)
    }

    // ========================================================================
    // Helpers
    // ========================================================================

    pub(crate) fn convert_span(&self, core_span: CoreSpan) -> Span {
        Span {
            start: core_span.start,
            end: core_span.end,
        }
    }

    pub(crate) fn emit(&mut self, kind: MirInstrKind, span: Span) {
        if let Some(f) = &mut self.current_func {
            if let Some(block) = f.blocks.get_mut(self.current_block) {
                block.instructions.push(MirInstr { kind, span });
            }
        }
    }

    pub(crate) fn set_terminator(&mut self, term: MirTerminator) {
        if let Some(f) = &mut self.current_func {
            if let Some(block) = f.blocks.get_mut(self.current_block) {
                block.terminator = term;
            }
        }
    }

    pub(crate) fn add_block(&mut self, label: &str) {
        if let Some(f) = &mut self.current_func {
            f.blocks.push(MirBlock::new(label.to_string()));
            self.current_block = f.blocks.len() - 1;
        }
    }

    pub(crate) fn new_temp(&mut self) -> String {
        let name = format!("_t{}", self.temp_counter);
        self.temp_counter += 1;
        name
    }

    pub(crate) fn new_block_label(&mut self, prefix: &str) -> String {
        let label = format!("{}_{}", prefix, self.block_counter);
        self.block_counter += 1;
        label
    }

    pub(crate) fn expr_to_name(&self, expr: &HirExpr) -> String {
        match &expr.kind {
            HirExprKind::Local { name } => name.clone(),
            HirExprKind::Global { name } => name.clone(),
            _ => "__expr".to_string(),
        }
    }

    pub(crate) fn const_to_mir(&self, cv: &ConstValue) -> MirConst {
        match cv {
            ConstValue::Int(v) => MirConst::Int(*v),
            ConstValue::Float(v) => MirConst::Float(*v),
            ConstValue::Bool(v) => MirConst::Bool(*v),
            ConstValue::Str(v) => MirConst::Str(v.clone()),
            ConstValue::Nil => MirConst::Nil,
        }
    }

    pub(crate) fn binop_to_mir(&self, op: HirBinOp) -> BinaryOp {
        match op {
            HirBinOp::Add => BinaryOp::Add,
            HirBinOp::Sub => BinaryOp::Sub,
            HirBinOp::Mul => BinaryOp::Mul,
            HirBinOp::Div => BinaryOp::Div,
            HirBinOp::Mod => BinaryOp::Mod,
            HirBinOp::Eq => BinaryOp::Eq,
            HirBinOp::NotEq => BinaryOp::Ne,
            HirBinOp::Lt => BinaryOp::Lt,
            HirBinOp::Gt => BinaryOp::Gt,
            HirBinOp::LtEq => BinaryOp::Le,
            HirBinOp::GtEq => BinaryOp::Ge,
            HirBinOp::In => BinaryOp::Eq,
            HirBinOp::And => BinaryOp::And,
            HirBinOp::Or => BinaryOp::Or,
            HirBinOp::BitAnd => BinaryOp::And,
            HirBinOp::BitOr => BinaryOp::Or,
        }
    }

    pub(crate) fn infer_container_kind(&self, expr: &HirExpr) -> Option<ContainerKind> {
        match &expr.kind {
            HirExprKind::Array(_) => Some(ContainerKind::Array),
            HirExprKind::Map(_) => Some(ContainerKind::Map),
            HirExprKind::Move(inner) | HirExprKind::Clone(inner) => self.infer_container_kind(inner),
            HirExprKind::Borrow { expr: inner, .. } => self.infer_container_kind(inner),
            HirExprKind::Local { name } => self.container_kinds.get(name).copied(),
            _ => {
                expr.type_id
                    .and_then(|tid| self.container_kind_from_type_id(tid))
            }
        }
    }

    pub(crate) fn container_kind_from_expr(&self, expr: &HirExpr) -> Option<ContainerKind> {
        self.infer_container_kind(expr)
    }

    fn container_kind_from_type_id(&self, type_id: CoreTypeId) -> Option<ContainerKind> {
        self.type_registry.get(type_id).and_then(|info| match info.kind {
            TypeKind::Array { .. } => Some(ContainerKind::Array),
            TypeKind::Map { .. } => Some(ContainerKind::Map),
            _ => None,
        })
    }

    pub(crate) fn array_elem_type_from_type_id(&self, type_id: CoreTypeId) -> Option<CoreTypeId> {
        self.type_registry.get(type_id).and_then(|info| match info.kind {
            TypeKind::Array { element } => Some(element),
            _ => None,
        })
    }

    pub(crate) fn map_types_from_type_id(&self, type_id: CoreTypeId) -> Option<(CoreTypeId, CoreTypeId)> {
        self.type_registry.get(type_id).and_then(|info| match info.kind {
            TypeKind::Map { key, value } => Some((key, value)),
            _ => None,
        })
    }

    pub(crate) fn unaryop_to_mir(&self, op: HirUnaryOp) -> UnaryOp {
        match op {
            HirUnaryOp::Neg => UnaryOp::Neg,
            HirUnaryOp::Not => UnaryOp::Not,
        }
    }
}
