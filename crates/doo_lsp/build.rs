/// Pre-link build script: renames the previous doo-lsp binary before linking.
///
/// On Windows, running .exe files are locked and can't be overwritten.
/// This build.rs renames the old binary before the linker writes the new one.
/// On Linux/macOS this is harmless but consistent.
fn main() {
    // Only re-run when build.rs itself changes (normal cargo behavior).
    println!("cargo:rerun-if-changed=build.rs");

    if let Ok(out_dir) = std::env::var("OUT_DIR") {
        // OUT_DIR = <target-dir>/<profile>/build/doo_lsp-<hash>/out
        // Navigate up 3 levels to reach the profile directory
        let profile_dir = std::path::Path::new(&out_dir)
            .ancestors()
            .nth(3)
            .unwrap_or(std::path::Path::new("."));

        let binary_name = if cfg!(target_os = "windows") {
            "doo-lsp.exe"
        } else {
            "doo-lsp"
        };

        let old_name = format!("{}.old", binary_name);

        let binary = profile_dir.join(binary_name);
        let old = profile_dir.join(&old_name);

        // Clean up leftover .old from previous builds
        let _ = std::fs::remove_file(&old);

        // Rename current binary so linker can write the new one
        if binary.exists() {
            let _ = std::fs::rename(&binary, &old);
            let _ = std::fs::remove_file(&old);
        }
    }
}
