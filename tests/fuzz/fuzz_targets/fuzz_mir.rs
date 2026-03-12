#![no_main]
use doo_frontend::Parser;
use doo_hir::Lower;
use doo_mir::builder::MirBuilder;
use doo_core::types::TypeRegistry;
use libfuzzer_sys::fuzz_target;
use std::sync::Arc;

fuzz_target!(|data: &[u8]| {
    // Skip empty inputs
    if data.is_empty() {
        return;
    }

    // Skip very large inputs to prevent stack overflow during MIR building
    // MIR builder performs deep recursion
    if data.len() > 1024 {
        return;
    }

    // Only process valid UTF-8 inputs
    if let Ok(s) = std::str::from_utf8(data) {
        let mut parser = Parser::new(s, 0);
        if let Ok(program) = parser.parse_program() {
                let mut analyzer = SemanticAnalyzer::new(None);
                if analyzer.analyze_program(nodes).is_ok() {
                    let mut mir_builder = MirBuilder::new();
                    mir_builder.build_program(nodes);
                    mir_builder.finalize();
                }
            }
        }
    }
});
