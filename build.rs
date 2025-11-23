use std::fs;
use std::path::PathBuf;

fn main() {
    // Get the target directory from the environment
    let target_dir = PathBuf::from(std::env::var("TARGET_DIR").unwrap_or_else(|_| {
        // Fallback: construct from OUT_DIR
        let out_dir = std::env::var("OUT_DIR").unwrap_or_default();
        let path = PathBuf::from(out_dir);
        // OUT_DIR is like target/debug/build/doo-xxx/out
        // We want target/debug
        path.ancestors()
            .nth(3)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "target/debug".to_string())
    }));

    // Get the current directory (project root)
    let current_dir = std::env::current_dir().expect("Failed to get current directory");

    // Source FFI libraries
    let ffi_libs_source = current_dir
        .join("ffi_libs")
        .join("libdoo_file")
        .join("target")
        .join("release");

    // Target directories
    let debug_target = current_dir.join("target").join("debug");
    let release_target = current_dir.join("target").join("release");

    // Files to copy
    let files_to_copy = vec![
        "doo_file.dll",
        "doo_file.dll.lib",
        "doo_file.dll.exp",
        "doo_file.pdb",
    ];

    // Copy to debug directory
    if debug_target.exists() {
        for file in &files_to_copy {
            let src = ffi_libs_source.join(file);
            let dst = debug_target.join(file);

            if src.exists() {
                if let Err(e) = fs::copy(&src, &dst) {
                    eprintln!("Warning: Failed to copy {} to debug: {}", file, e);
                }
            }
        }
    }

    // Copy to release directory
    if release_target.exists() {
        for file in &files_to_copy {
            let src = ffi_libs_source.join(file);
            let dst = release_target.join(file);

            if src.exists() {
                if let Err(e) = fs::copy(&src, &dst) {
                    eprintln!("Warning: Failed to copy {} to release: {}", file, e);
                }
            }
        }
    }

    // Rebuild on changes to FFI libraries
    println!("cargo:rerun-if-changed=ffi_libs/libdoo_file/src/lib.rs");
    println!("cargo:rerun-if-changed=ffi_libs/libdoo_file/target/release/doo_file.dll");
}
