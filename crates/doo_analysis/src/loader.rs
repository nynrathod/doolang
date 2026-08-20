// Add after the existing code in loader.rs:

use rustc_hash::FxHashMap;
use std::collections::HashSet;

/// Module graph for tracking import dependencies.
///
/// Phase 23.1: Build module graph from use imports.
/// Detect circular imports at graph build time.
#[derive(Debug, Default)]
pub struct ModuleGraph {
    /// Adjacency list: module path -> imported module paths.
    edges: FxHashMap<String, Vec<String>>,
    /// All known module paths.
    modules: HashSet<String>,
}

impl ModuleGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a module to the graph.
    pub fn add_module(&mut self, path: &str) {
        self.modules.insert(path.to_string());
        self.edges.entry(path.to_string()).or_default();
    }

    /// Add an import edge from one module to another.
    pub fn add_edge(&mut self, from: &str, to: &str) {
        self.modules.insert(from.to_string());
        self.modules.insert(to.to_string());
        self.edges
            .entry(from.to_string())
            .or_default()
            .push(to.to_string());
    }

    /// Detect circular imports using DFS.
    /// Returns the cycle path if a cycle is found.
    pub fn detect_cycles(&self) -> Option<Vec<String>> {
        let mut visited: HashSet<&str> = HashSet::new();
        let mut in_stack: HashSet<&str> = HashSet::new();

        let mut modules: Vec<&String> = self.modules.iter().collect();
        modules.sort();

        for module in &modules {
            if !visited.contains(module.as_str()) {
                let mut path = Vec::new();
                if let Some(cycle) = self.dfs_cycle(module, &mut visited, &mut in_stack, &mut path)
                {
                    return Some(cycle);
                }
            }
        }
        None
    }

    fn dfs_cycle<'a>(
        &'a self,
        module: &'a str,
        visited: &mut HashSet<&'a str>,
        in_stack: &mut HashSet<&'a str>,
        path: &mut Vec<&'a str>,
    ) -> Option<Vec<String>> {
        visited.insert(module);
        in_stack.insert(module);
        path.push(module);

        if let Some(edges) = self.edges.get(module) {
            for target in edges {
                if !visited.contains(target.as_str()) {
                    if let Some(cycle) = self.dfs_cycle(target, visited, in_stack, path) {
                        return Some(cycle);
                    }
                } else if in_stack.contains(target.as_str()) {
                    // Found cycle
                    let start = path.iter().position(|&m| m == target).unwrap();
                    let mut cycle: Vec<String> =
                        path[start..].iter().map(|s| s.to_string()).collect();
                    cycle.push(target.to_string());
                    return Some(cycle);
                }
            }
        }

        in_stack.remove(module);
        path.pop();
        None
    }

    /// Get all modules that a given module imports.
    pub fn imports_of(&self, module: &str) -> Option<&[String]> {
        self.edges.get(module).map(|v| v.as_slice())
    }

    /// Check if a module exists in the graph.
    pub fn has_module(&self, module: &str) -> bool {
        self.modules.contains(module)
    }
}

#[cfg(test)]
mod module_graph_tests {
    use super::*;

    #[test]
    fn test_no_cycle() {
        let mut graph = ModuleGraph::new();
        graph.add_edge("a", "b");
        graph.add_edge("b", "c");
        assert!(graph.detect_cycles().is_none());
    }

    #[test]
    fn test_simple_cycle() {
        let mut graph = ModuleGraph::new();
        graph.add_edge("a", "b");
        graph.add_edge("b", "a");
        let cycle = graph.detect_cycles().unwrap();
        assert!(cycle.len() >= 3); // a, b, a
    }

    #[test]
    fn test_indirect_cycle() {
        let mut graph = ModuleGraph::new();
        graph.add_edge("a", "b");
        graph.add_edge("b", "c");
        graph.add_edge("c", "a");
        let cycle = graph.detect_cycles().unwrap();
        assert!(cycle.len() >= 4); // a, b, c, a
    }
}
