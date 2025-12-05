use std::fs;

fn main() {
    // Get the current directory (project root)
    let current_dir = std::env::current_dir().expect("Failed to get current directory");

    // Source FFI libraries directory
    let ffi_libs_source = current_dir
        .join("ffi_libs")
        .join("libdoo_file")
        .join("target")
        .join("release");

    // Target directories
    let debug_target = current_dir.join("target").join("debug");
    let release_target = current_dir.join("target").join("release");

    // Platform-specific files to copy
    #[cfg(target_os = "windows")]
    let files_to_copy = vec![
        "doo_file.dll",
        "doo_file.dll.lib",
        "doo_file.dll.exp",
        "doo_file.pdb",
    ];

    #[cfg(target_os = "linux")]
    let files_to_copy = vec!["libdoo_file.so"];

    #[cfg(target_os = "macos")]
    let files_to_copy = vec!["libdoo_file.dylib"];

    // Copy files to both debug and release directories
    for target_dir in [&debug_target, &release_target] {
        if target_dir.exists() {
            for file in &files_to_copy {
                let src = ffi_libs_source.join(file);
                let dst = target_dir.join(file);

                if src.exists() {
                    if let Err(e) = fs::copy(&src, &dst) {
                        eprintln!(
                            "Warning: Failed to copy {} to {:?}: {}",
                            file, target_dir, e
                        );
                    }
                }
            }
        }
    }

    // Rebuild on changes to FFI libraries
    println!("cargo:rerun-if-changed=ffi_libs/libdoo_file/src/lib.rs");

    #[cfg(target_os = "windows")]
    println!("cargo:rerun-if-changed=ffi_libs/libdoo_file/target/release/doo_file.dll");

    #[cfg(target_os = "linux")]
    println!("cargo:rerun-if-changed=ffi_libs/libdoo_file/target/release/libdoo_file.so");

    #[cfg(target_os = "macos")]
    println!("cargo:rerun-if-changed=ffi_libs/libdoo_file/target/release/libdoo_file.dylib");
}
