#![no_main]
use doo_frontend::Lexer;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Skip empty inputs
    if data.is_empty() {
        return;
    }

    // Skip very large inputs to prevent OOM
    // Lexer uses arena which is freed after this scope
    if data.len() > 1024 {
        return;
    }

    // Only process valid UTF-8 inputs
    if let Ok(s) = std::str::from_utf8(data) {
        let tokens = lex(s, &arena);

        // Additional safety: bail if token explosion occurs
        // Arena automatically frees when it goes out of scope
        if tokens.len() > 5000 {
            return;
        }
    }
});
