//! Build script for doo_driver.
//!
//! Ensures the correct LLVM-C.dll is available next to the compiled binary.
//! On Windows, the system PATH may have a different LLVM version installed,
//! which would cause access violations at runtime due to ABI mismatch.
//!
//! This build script reads the LLVM installation prefix (the same env var
//! used by llvm-sys during compilation) and copies the matching DLL to
//! the output directory, ensuring runtime linkage against the correct version.

use std::env;
use std::path::PathBuf;

fn main() {
    // Only needed on Windows — other platforms link statically or use system libs
    if env::var("CARGO_CFG_TARGET_OS").map_or(false, |os| os != "windows") {
        return;
    }

    // Read the LLVM prefix from the same env var that llvm-sys uses.
    // llvm-sys 221 uses LLVM_SYS_221_PREFIX; older versions use LLVM_SYS_PREFIX.
    // Try the versioned var first, then the generic one.
    let llvm_prefix = env::var("LLVM_SYS_221_PREFIX").or_else(|_| env::var("LLVM_SYS_PREFIX"));

    let llvm_prefix = match llvm_prefix {
        Ok(p) => PathBuf::from(p),
        Err(_) => {
            // LLVM prefix not set — llvm-sys will auto-detect.
            // We can't copy the DLL without knowing the source path.
            // Emit a warning but don't fail the build.
            println!(
                "cargo:warning=LLVM_SYS_221_PREFIX not set — cannot copy LLVM-C.dll automatically"
            );
            println!("cargo:warning=Ensure the correct LLVM-C.dll (matching llvm-sys version) is on PATH at runtime");
            return;
        }
    };

    let dll_name = "LLVM-C.dll";
    let dll_src = llvm_prefix.join("bin").join(dll_name);

    if !dll_src.exists() {
        println!(
            "cargo:warning=LLVM-C.dll not found at {}",
            dll_src.display()
        );
        println!("cargo:warning=Ensure the correct LLVM-C.dll is on PATH at runtime");
        return;
    }

    // Determine the output directory where the binary will be placed.
    // OUT_DIR points to target/<profile>/build/<crate>-<hash>/out
    // Navigate up to the profile directory (e.g., target/release/).
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    // out_dir = target-windows/release/build/doo_driver-<hash>/out
    // Go up 3 levels: out -> doo_driver-<hash> -> build -> release
    let profile_dir = out_dir
        .parent() // out
        .and_then(|p| p.parent()) // doo_driver-<hash>
        .and_then(|p| p.parent()) // build
        .map(|p| p.to_path_buf());

    let profile_dir = match profile_dir {
        Some(d) => d,
        None => {
            println!(
                "cargo:warning=Could not determine output directory from OUT_DIR={}",
                out_dir.display()
            );
            return;
        }
    };

    let dll_dst = profile_dir.join(dll_name);

    // Only copy if the source is newer or the destination doesn't exist
    let should_copy = !dll_dst.exists()
        || dll_src.metadata().ok().and_then(|m| m.modified().ok())
            > dll_dst.metadata().ok().and_then(|m| m.modified().ok());

    if should_copy {
        match std::fs::copy(&dll_src, &dll_dst) {
            Ok(_) => {
                println!(
                    "cargo:warning=Copied {} -> {}",
                    dll_src.display(),
                    dll_dst.display()
                );
            }
            Err(e) => {
                println!(
                    "cargo:warning=Failed to copy {} -> {}: {}",
                    dll_src.display(),
                    dll_dst.display(),
                    e
                );
                println!("cargo:warning=Ensure the correct LLVM-C.dll is on PATH at runtime");
            }
        }
    }

    // Also tell cargo to rerun this script if the env var or DLL changes
    println!("cargo:rerun-if-env-changed=LLVM_SYS_221_PREFIX");
    println!("cargo:rerun-if-env-changed=LLVM_SYS_PREFIX");
}
