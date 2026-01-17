fn main() {
    // Get the target directory - respects CARGO_TARGET_DIR environment variable
    let target_dir = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| {
        // Fallback: use default target directory relative to project root
        // CARGO_MANIFEST_DIR is ffi_libs/libdoo_http, so we go up 2 levels to reach project root
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        format!("{}/../../target", manifest_dir)
    });

    // On Windows, link against the .dll.lib import libraries using full paths
    #[cfg(target_os = "windows")]
    {
        println!(
            "cargo:rustc-link-arg={}\\release\\doo_runtime.dll.lib",
            target_dir
        );
        println!(
            "cargo:rustc-link-arg={}\\release\\doo_db.dll.lib",
            target_dir
        );
        println!(
            "cargo:rustc-link-arg={}\\release\\doo_auth.dll.lib",
            target_dir
        );
    }

    // On Linux
    #[cfg(target_os = "linux")]
    {
        println!("cargo:rustc-link-search=native={}/release", target_dir);
        println!("cargo:rustc-link-lib=dylib=doo_runtime");
        println!("cargo:rustc-link-lib=dylib=doo_db");
        println!("cargo:rustc-link-lib=dylib=doo_auth");
    }

    // On macOS
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-search=native={}/release", target_dir);
        println!("cargo:rustc-link-lib=dylib=doo_runtime");
        println!("cargo:rustc-link-lib=dylib=doo_db");
        println!("cargo:rustc-link-lib=dylib=doo_auth");
    }
}
