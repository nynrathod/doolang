//! Type Inference
//!
//! Infers types for expressions and statements.

use doo_core::types::{TypeId, builtin};

/// Type inference engine.
pub struct TypeInference {
    /// Type constraints collected during inference.
    constraints: Vec<TypeConstraint>,
}

/// A type constraint.
#[derive(Debug, Clone)]
pub struct TypeConstraint {
    pub lhs: TypeId,
    pub rhs: TypeId,
}

impl TypeInference {
    pub fn new() -> Self {
        Self {
            constraints: Vec::new(),
        }
    }

    /// Add a constraint: lhs must be compatible with rhs.
    pub fn constrain(&mut self, lhs: TypeId, rhs: TypeId) {
        self.constraints.push(TypeConstraint { lhs, rhs });
    }

    /// Solve constraints and return unified types.
    pub fn solve(&self) -> Result<(), InferenceError> {
        // Simple unification - full implementation would use union-find
        for c in &self.constraints {
            if !self.unify(c.lhs, c.rhs) {
                return Err(InferenceError::Mismatch(c.lhs, c.rhs));
            }
        }
        Ok(())
    }

    fn unify(&self, a: TypeId, b: TypeId) -> bool {
        // Same type always unifies
        if a == b {
            return true;
        }
        // ANY unifies with everything
        if a == builtin::ANY || b == builtin::ANY {
            return true;
        }
        false
    }
}

impl Default for TypeInference {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub enum InferenceError {
    Mismatch(TypeId, TypeId),
}
