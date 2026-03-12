#[macro_export]
macro_rules! doo_debug {
    ($($arg:tt)*) => {
        if cfg!(debug_assertions) || std::env::var("DOO_DEBUG").is_ok() {
             eprintln!("[COMPILER] {}", format!($($arg)*));
        }
    }
}
