#![no_main]
use doo_frontend::Parser;
use doo_hir::Lower;
use doo_analysis::TypeChecker;
use doo_core::types::TypeRegistry;
use libfuzzer_sys::fuzz_target;
use std::sync::Arc;

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    if data.len() > 1024 {
        return;
    }

    if let Ok(s) = std::str::from_utf8(data) {
        let mut parser = Parser::new(s, 0);
        if let Ok(program) = parser.parse_program() {

        let mut parser = Parser::new(&tokens);
        if let Ok(mut ast) = parser.parse_program() {
            if let AstNode::Program(ref mut nodes) = ast {
                let mut analyzer = SemanticAnalyzer::new(None);
                let _ = analyzer.analyze_program(nodes);
            }
        }
    }
});
