//! Display implementations for types.
//!
//! Provides human-readable formatting for all type kinds.

use super::{TypeId, TypeKind, CollectionType, CompositeType};

impl std::fmt::Display for TypeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "T{}", self.0)
    }
}

impl std::fmt::Display for TypeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeKind::Void => write!(f, "Void"),
            TypeKind::Bool => write!(f, "Bool"),
            TypeKind::Int => write!(f, "Int"),
            TypeKind::Float => write!(f, "Float"),
            TypeKind::Str => write!(f, "Str"),
            TypeKind::Array { element } => write!(f, "[{}]", element),
            TypeKind::Map { key, value } => write!(f, "Map<{}, {}>", key, value),
            TypeKind::Optional { inner } => write!(f, "{}?", inner),
            TypeKind::Result { ok, err } => write!(f, "Result<{}, {}>", ok, err),
            TypeKind::Tuple { elements } => {
                write!(f, "(")?;
                for (i, e) in elements.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", e)?;
                }
                write!(f, ")")
            }
            TypeKind::Struct { name, .. } => write!(f, "{}", name),
            TypeKind::Enum { name, .. } => write!(f, "{}", name),
            TypeKind::Interface { name, .. } => write!(f, "{}", name),
            TypeKind::Function { params, returns } => {
                write!(f, "fn(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                write!(f, ") -> {}", returns)
            }
            TypeKind::TypeRef { name } => write!(f, "{}", name),
            TypeKind::Any => write!(f, "Any"),
            TypeKind::Error => write!(f, "Error"),
            TypeKind::TypeParam { name } => write!(f, "{}", name),
        }
    }
}

impl std::fmt::Display for CollectionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Array { element } => write!(f, "[{}]", element),
            Self::Map { key, value } => write!(f, "Map<{}, {}>", key, value),
            Self::Tuple { elements } => {
                write!(f, "(")?;
                for (i, e) in elements.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", e)?;
                }
                write!(f, ")")
            }
            Self::Range { element, inclusive } => {
                let op = if *inclusive { "..=" } else { ".." };
                write!(f, "Range<{}>{}", element, op)
            }
            Self::Optional { inner } => write!(f, "{}?", inner),
            Self::Result { ok, err } => write!(f, "Result<{}, {}>", ok, err),
        }
    }
}

impl std::fmt::Display for CompositeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}
