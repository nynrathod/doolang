use std::fs;

fn main() {
    // Get the current directory (project root)
    let current_dir = std::env::current_dir().expect("Failed to get current directory");

    // FFI libraries to copy
    let ffi_libraries = vec!["libdoo_file", "libdoo_runtime"];

    // Target directories
    let debug_target = current_dir.join("target").join("debug");
    let release_target = current_dir.join("target").join("release");

    for lib_name in &ffi_libraries {
        // Source FFI library directory
        let ffi_libs_source = current_dir
            .join("ffi_libs")
            .join(lib_name)
            .join("target")
            .join("release");

        // Extract the library name without "lib" prefix for the output files
        let lib_short_name = lib_name.strip_prefix("lib").unwrap_or(lib_name);

        // Platform-specific files to copy
        #[cfg(target_os = "windows")]
        let files_to_copy = vec![
            format!("{}.dll", lib_short_name),
            format!("{}.dll.lib", lib_short_name),
            format!("{}.dll.exp", lib_short_name),
            format!("{}.pdb", lib_short_name),
        ];

        #[cfg(target_os = "linux")]
        let files_to_copy = vec![format!("lib{}.so", lib_short_name)];

        #[cfg(target_os = "macos")]
        let files_to_copy = vec![format!("lib{}.dylib", lib_short_name)];

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
        println!("cargo:rerun-if-changed=ffi_libs/{}/src/lib.rs", lib_name);

        #[cfg(target_os = "windows")]
        println!(
            "cargo:rerun-if-changed=ffi_libs/{}/target/release/{}.dll",
            lib_name, lib_short_name
        );

        #[cfg(target_os = "linux")]
        println!(
            "cargo:rerun-if-changed=ffi_libs/{}/target/release/lib{}.so",
            lib_name, lib_short_name
        );

        #[cfg(target_os = "macos")]
        println!(
            "cargo:rerun-if-changed=ffi_libs/{}/target/release/lib{}.dylib",
            lib_name, lib_short_name
        );
    }
}
