#![no_main]
use bumpalo::Bump;
use doo::analyzer::SemanticAnalyzer;
use doo::lexer::lexer::lex;
use doo::mir::builder::MirBuilder;
use doo::parser::ast::AstNode;
use doo::parser::Parser;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Skip empty inputs
    if data.is_empty() {
        return;
    }

    // Skip very large inputs to prevent stack overflow during MIR building
    // MIR builder performs deep recursion, and lexer uses arena now, so use very tight limit
    if data.len() > 1024 {
        return;
    }

    // Only process valid UTF-8 inputs
    if let Ok(s) = std::str::from_utf8(data) {
        let arena = Bump::new();
        let tokens = lex(s, &arena);

        // Additional safety: bail if token explosion occurs
        // Arena automatically frees when it goes out of scope
        if tokens.len() > 5000 {
            return;
        }

        let mut parser = Parser::new(&tokens);
        if let Ok(mut ast) = parser.parse_program() {
            if let AstNode::Program(ref mut nodes) = ast {
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
