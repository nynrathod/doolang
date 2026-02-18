#![no_main]
use doo_frontend::{Lexer, Parser};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    if data.len() > 1024 {
        return;
    }

    if let Ok(s) = std::str::from_utf8(data) {
        let mut lexer = Lexer::new(s, 0);
        let tokens = lexer.tokenize();

        if tokens.len() > 5000 {
            return;
        }

        let mut parser = Parser::new(&tokens);
        let _ = parser.parse_program();
    }
});
