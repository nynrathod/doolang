#![no_main]
use bumpalo::Bump;
use doo::analyzer::SemanticAnalyzer;
use doo::lexer::lexer::lex;
use doo::parser::ast::AstNode;
use doo::parser::Parser;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    if data.len() > 1024 {
        return;
    }

    if let Ok(s) = std::str::from_utf8(data) {
        let arena = Bump::new();
        let tokens = lex(s, &arena);

        if tokens.len() > 5000 {
            return;
        }

        let mut parser = Parser::new(&tokens);
        if let Ok(mut ast) = parser.parse_program() {
            if let AstNode::Program(ref mut nodes) = ast {
                let mut analyzer = SemanticAnalyzer::new(None);
                let _ = analyzer.analyze_program(nodes);
            }
        }
    }
});
