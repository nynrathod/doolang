/// Simple runtime panic function for error handling
#[no_mangle]
pub extern "C" fn panic_runtime(msg: *const u8) {
    if msg.is_null() {
        eprintln!("Runtime error: panic");
        std::process::exit(1);
    }

    // Convert C string to Rust string
    unsafe {
        let c_str = std::ffi::CStr::from_ptr(msg as *const i8);
        if let Ok(msg_str) = c_str.to_str() {
            eprintln!("{}", msg_str);
        } else {
            eprintln!("Runtime error: invalid UTF-8 in panic message");
        }
    }

    std::process::exit(1);
}
