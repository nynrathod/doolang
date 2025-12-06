// No build script logic needed!
// The Cargo workspace automatically builds all members when you run `cargo build`.
// FFI libraries will be in target/{profile}/deps/ and the linker will find them.

fn main() {
    // Only rerun if these change (for cache efficiency)
    println!("cargo:rerun-if-changed=ffi_libs/libdoo_file/src");
    println!("cargo:rerun-if-changed=ffi_libs/libdoo_file/Cargo.toml");
}
