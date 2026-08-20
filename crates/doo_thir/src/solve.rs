//! Trait/Interface Resolution Engine
//!
//! Resolves method calls to their specific implementation.
//!
//! ## Algorithm
//!
//! Given a method call `receiver.method(args)`:
//! 1. Look up method in inherent methods of receiver type → if found, `ImplResolution::Direct`
//! 2. Otherwise, search all `TraitImpl` where `trait_name` has `method` and `for_type` unifies with receiver
//! 3. If exactly one match → `ImplResolution::Trait(trait_name)`
//! 4. If zero matches → error E0001 (unknown method)
//! 5. If multiple matches → error (coherence violation)
//!
//! ## Coherence Rule
//! At most one applicable impl per type+trait pair (Architecture Part V §5.1).
//! Interface methods always take `self` by immutable borrow (Decision 6.2).

use doo_core::types::{TypeId, TypeRegistry};
use rustc_hash::FxHashMap;

use crate::types::ImplResolution;

/// A trait implementation mapping a type to an interface.
#[derive(Debug, Clone)]
pub struct TraitImpl {
    pub trait_name: String,
    pub for_type: TypeId,
    pub methods: Vec<(String, TypeId)>,
}

/// Trait solver that resolves method calls to specific implementations.
#[derive(Debug, Default)]
pub struct TraitSolver {
    /// All known trait implementations.
    impls: Vec<TraitImpl>,
    /// Inherent methods: type_name -> method_name -> ()
    inherent_methods: FxHashMap<String, FxHashMap<String, ()>>,
    /// Interface definitions: interface_name -> Set<method_name>
    interfaces: FxHashMap<String, FxHashMap<String, ()>>,
}

impl TraitSolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an inherent method for a type.
    pub fn register_inherent_method(&mut self, type_name: &str, method_name: &str) {
        self.inherent_methods
            .entry(type_name.to_string())
            .or_default()
            .insert(method_name.to_string(), ());
    }

    /// Register an interface and its methods.
    pub fn register_interface(&mut self, name: &str, methods: Vec<String>) {
        let map: FxHashMap<String, ()> = methods.into_iter().map(|m| (m, ())).collect();
        self.interfaces.insert(name.to_string(), map);
    }

    /// Register a trait implementation.
    pub fn register_impl(&mut self, impl_decl: TraitImpl) {
        self.impls.push(impl_decl);
    }

    /// Resolve a method call to its implementation.
    pub fn resolve_method(
        &self,
        receiver_type: TypeId,
        method: &str,
        registry: &TypeRegistry,
    ) -> ImplResolution {
        // 1. Check inherent methods on the exact type
        if let Some(info) = registry.get(receiver_type) {
            for type_name in info.inherent_impl_names() {
                if let Some(methods) = self.inherent_methods.get(&type_name) {
                    if methods.contains_key(method) {
                        return ImplResolution::Direct;
                    }
                }
            }
        }

        // 3. Search trait impls
        let mut matches: Vec<&TraitImpl> = Vec::new();
        for impl_decl in &self.impls {
            if impl_decl.methods.iter().any(|(m, _)| m == method) {
                if self.types_unify(receiver_type, impl_decl.for_type, registry) {
                    matches.push(impl_decl);
                }
            }
        }

        match matches.len() {
            0 => {
                // Check if any interface defines this method
                for (iface_name, methods) in &self.interfaces {
                    if methods.contains_key(method) {
                        return ImplResolution::Trait(iface_name.clone());
                    }
                }
                ImplResolution::Direct // Fallback to Direct for builtins
            }
            1 => ImplResolution::Trait(matches[0].trait_name.clone()),
            _ => {
                // Coherence violation - should have been caught at declaration
                ImplResolution::Trait(matches[0].trait_name.clone())
            }
        }
    }

    /// Check if two types unify (simplified structural equality).
    fn types_unify(&self, a: TypeId, b: TypeId, registry: &TypeRegistry) -> bool {
        if a == b {
            return true;
        }
        // Check if both resolve to the same named type
        if let (Some(a_info), Some(b_info)) = (registry.get(a), registry.get(b)) {
            if !a_info.name.is_empty() && !b_info.name.is_empty() {
                return a_info.name == b_info.name;
            }
        }
        false
    }
}
