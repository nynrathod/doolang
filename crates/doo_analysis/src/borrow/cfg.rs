//! Control Flow Graph for borrow checking.
//!
//! Built from THIR function bodies. Each basic block contains a sequence
//! of statements and a single terminator that transfers control to
//! successor blocks.

use doo_core::types::TypeId;
use doo_core::Span;
use doo_thir::{ThirArm, ThirExpr, ThirExprKind, ThirFunction, ThirStmt, ThirStmtKind};

/// Identifier for a basic block in the CFG.
pub type BlockId = u32;

/// A control flow graph built from a THIR function body.
pub struct CFG {
    /// All basic blocks, indexed by BlockId.
    pub blocks: Vec<BasicBlock>,
    /// Entry block ID.
    pub entry: BlockId,
    /// Exit block ID.
    pub exit: BlockId,
}

/// A basic block: straight-line statements ending with a terminator.
pub struct BasicBlock {
    /// Statements in this block (executed in order).
    pub stmts: Vec<ThirStmt>,
    /// How control leaves this block.
    pub terminator: Terminator,
    /// Predecessor blocks (for dataflow join).
    pub preds: Vec<BlockId>,
    /// Successor blocks (for dataflow propagation).
    pub succs: Vec<BlockId>,
}

/// Control flow terminator for a basic block.
#[derive(Debug, Clone)]
pub enum Terminator {
    /// Unconditional jump to another block.
    Goto(BlockId),
    /// Conditional branch.
    If {
        cond: ThirExpr,
        then_block: BlockId,
        else_block: BlockId,
    },
    /// Match dispatch.
    Match {
        expr: ThirExpr,
        arms: Vec<(BlockId)>,
        default: Option<BlockId>,
    },
    /// Return from the function.
    Return(Option<ThirExpr>),
    /// Break from a loop (with optional value).
    Break(Option<ThirExpr>),
    /// Continue a loop.
    Continue,
    /// Function exit (no value).
    Exit,
}

/// Builds a CFG from a THIR function body.
pub struct CfgBuilder {
    blocks: Vec<BasicBlock>,
}

impl CfgBuilder {
    pub fn new() -> Self {
        Self { blocks: Vec::new() }
    }

    /// Build a CFG from a THIR function.
    pub fn build(mut self, func: &ThirFunction) -> CFG {
        let entry = self.new_block();
        let exit = self.new_block();

        self.build_stmts(&func.body, entry, exit);

        // Ensure entry block's terminator is set
        if self.blocks[entry as usize].preds.is_empty() {
            // entry has no preds — it's the start
        }

        // Link any block with Exit terminator to the exit block
        for i in 0..self.blocks.len() {
            let term = std::mem::replace(&mut self.blocks[i].terminator, Terminator::Goto(exit));
            match &term {
                Terminator::Return(_) | Terminator::Exit => {
                    self.blocks[i].terminator = Terminator::Goto(exit);
                    self.link(i as BlockId, exit);
                }
                _ => {
                    self.blocks[i].terminator = term;
                }
            }
        }

        // Compute successors from terminators
        let n = self.blocks.len();
        for i in 0..n {
            let succs = match &self.blocks[i].terminator {
                Terminator::Goto(target) => vec![*target],
                Terminator::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    vec![*then_block, *else_block]
                }
                Terminator::Match { arms, default, .. } => {
                    let mut s: Vec<BlockId> = arms.clone();
                    if let Some(d) = default {
                        s.push(*d);
                    }
                    s
                }
                Terminator::Return(_)
                | Terminator::Break(_)
                | Terminator::Continue
                | Terminator::Exit => {
                    vec![]
                }
            };
            for succ in &succs {
                self.blocks[*succ as usize].preds.push(i as BlockId);
            }
            self.blocks[i].succs = succs;
        }

        CFG {
            blocks: self.blocks,
            entry,
            exit,
        }
    }

    fn new_block(&mut self) -> BlockId {
        let id = self.blocks.len() as BlockId;
        self.blocks.push(BasicBlock {
            stmts: Vec::new(),
            terminator: Terminator::Exit,
            preds: Vec::new(),
            succs: Vec::new(),
        });
        id
    }

    fn link(&mut self, from: BlockId, to: BlockId) {
        if !self.blocks[from as usize].succs.contains(&to) {
            self.blocks[from as usize].succs.push(to);
        }
        if !self.blocks[to as usize].preds.contains(&from) {
            self.blocks[to as usize].preds.push(from);
        }
    }

    fn set_terminator(&mut self, block: BlockId, term: Terminator) {
        self.blocks[block as usize].terminator = term;
    }

    fn build_stmts(&mut self, stmts: &[ThirStmt], start: BlockId, exit: BlockId) {
        let mut current = start;

        for stmt in stmts {
            current = self.build_stmt(stmt, current, exit);
        }

        // If the last block doesn't have a terminator, link to exit
        if matches!(self.blocks[current as usize].terminator, Terminator::Exit) {
            self.set_terminator(current, Terminator::Goto(exit));
            self.link(current, exit);
        }
    }

    fn build_stmt(&mut self, stmt: &ThirStmt, current: BlockId, exit: BlockId) -> BlockId {
        match &stmt.kind {
            ThirStmtKind::Let { .. }
            | ThirStmtKind::Const { .. }
            | ThirStmtKind::Expr(_)
            | ThirStmtKind::Assign { .. }
            | ThirStmtKind::TupleLet { .. }
            | ThirStmtKind::ManualErrorExtract { .. }
            | ThirStmtKind::Drop { .. } => {
                self.blocks[current as usize].stmts.push(stmt.clone());
                current
            }

            ThirStmtKind::Return(opt_expr) => {
                self.set_terminator(current, Terminator::Return(opt_expr.clone()));
                self.link(current, exit);
                // Return is a dead end — create a new unreachable block for any following stmts
                self.new_block()
            }

            ThirStmtKind::Break(opt_expr) => {
                self.set_terminator(current, Terminator::Break(opt_expr.clone()));
                self.new_block()
            }

            ThirStmtKind::Continue => {
                self.set_terminator(current, Terminator::Continue);
                self.new_block()
            }

            ThirStmtKind::While {
                cond,
                body,
                increment,
            } => {
                let cond_block = self.new_block();
                let body_block = self.new_block();
                let inc_block = self.new_block();
                let after_block = self.new_block();

                // current → cond_block
                self.set_terminator(current, Terminator::Goto(cond_block));
                self.link(current, cond_block);

                // cond_block: if cond goto body else goto after
                self.set_terminator(
                    cond_block,
                    Terminator::If {
                        cond: cond.clone(),
                        then_block: body_block,
                        else_block: after_block,
                    },
                );
                self.link(cond_block, body_block);
                self.link(cond_block, after_block);

                // body_block → inc_block
                let body_end = self.build_stmts_inline(body, body_block);
                self.set_terminator(body_end, Terminator::Goto(inc_block));
                self.link(body_end, inc_block);

                // inc_block → cond_block (loop back)
                let inc_end = self.build_stmts_inline(increment, inc_block);
                self.set_terminator(inc_end, Terminator::Goto(cond_block));
                self.link(inc_end, cond_block);

                after_block
            }

            ThirStmtKind::Loop { body } => {
                let body_block = self.new_block();
                let after_block = self.new_block();

                self.set_terminator(current, Terminator::Goto(body_block));
                self.link(current, body_block);

                let body_end = self.build_stmts_inline(body, body_block);
                self.set_terminator(body_end, Terminator::Goto(body_block));
                self.link(body_end, body_block);

                after_block
            }

            ThirStmtKind::Go { expr } => {
                self.blocks[current as usize].stmts.push(stmt.clone());
                current
            }

            ThirStmtKind::Scope { stmts } => {
                let scope_start = self.new_block();
                self.set_terminator(current, Terminator::Goto(scope_start));
                self.link(current, scope_start);

                let scope_end = self.build_stmts_inline(stmts, scope_start);
                let after = self.new_block();
                self.set_terminator(scope_end, Terminator::Goto(after));
                self.link(scope_end, after);
                after
            }
        }
    }

    fn build_stmts_inline(&mut self, stmts: &[ThirStmt], start: BlockId) -> BlockId {
        let mut current = start;
        for stmt in stmts {
            // For inline building, we don't create an exit block
            match &stmt.kind {
                ThirStmtKind::Return(opt) => {
                    self.set_terminator(current, Terminator::Return(opt.clone()));
                    return self.new_block();
                }
                ThirStmtKind::Break(opt) => {
                    self.set_terminator(current, Terminator::Break(opt.clone()));
                    return self.new_block();
                }
                ThirStmtKind::Continue => {
                    self.set_terminator(current, Terminator::Continue);
                    return self.new_block();
                }
                ThirStmtKind::While { .. } | ThirStmtKind::Loop { .. } => {
                    // Nested loops need their own blocks
                    current = self.build_stmt(stmt, current, 0);
                }
                _ => {
                    self.blocks[current as usize].stmts.push(stmt.clone());
                }
            }
        }
        current
    }
}

impl CFG {
    /// Iterate over all block IDs.
    pub fn block_ids(&self) -> impl Iterator<Item = BlockId> + '_ {
        0..self.blocks.len() as BlockId
    }

    /// Get a block by ID.
    pub fn block(&self, id: BlockId) -> &BasicBlock {
        &self.blocks[id as usize]
    }

    /// Get a block mutably by ID.
    pub fn block_mut(&mut self, id: BlockId) -> &mut BasicBlock {
        &mut self.blocks[id as usize]
    }

    /// Get predecessors of a block.
    pub fn preds(&self, id: BlockId) -> &[BlockId] {
        &self.blocks[id as usize].preds
    }

    /// Get successors of a block.
    pub fn succs(&self, id: BlockId) -> &[BlockId] {
        &self.blocks[id as usize].succs
    }

    /// Number of blocks.
    pub fn num_blocks(&self) -> usize {
        self.blocks.len()
    }
}

impl Default for CfgBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cfg_builder_creation() {
        let builder = CfgBuilder::new();
        assert!(builder.blocks.is_empty());
    }

    #[test]
    fn test_empty_function() {
        let func = ThirFunction {
            name: "test".to_string(),
            type_params: vec![],
            params: vec![],
            return_type: None,
            error_type: None,
            body: vec![],
            is_async: false,
            span: Span::dummy(),
        };
        let cfg = CfgBuilder::new().build(&func);
        assert!(cfg.num_blocks() >= 2); // at least entry and exit
    }
}
