fn main() {
    // Get the absolute path to the target directory
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let target_dir = format!("{}/../..", manifest_dir);

    // On Windows, link against the .dll.lib import libraries using full paths
    #[cfg(target_os = "windows")]
    {
        println!(
            "cargo:rustc-link-arg={}\\target\\release\\doo_runtime.dll.lib",
            target_dir
        );
        println!(
            "cargo:rustc-link-arg={}\\target\\release\\doo_db.dll.lib",
            target_dir
        );
        println!(
            "cargo:rustc-link-arg={}\\target\\release\\doo_auth.dll.lib",
            target_dir
        );
    }

    // On Unix-like systems
    #[cfg(not(target_os = "windows"))]
    {
        println!(
            "cargo:rustc-link-search=native={}/target/release",
            target_dir
        );
        println!("cargo:rustc-link-lib=dylib=doo_runtime");
        println!("cargo:rustc-link-lib=dylib=doo_db");
        println!("cargo:rustc-link-lib=dylib=doo_auth");
    }

    // Rerun if the libraries change
    println!(
        "cargo:rerun-if-changed={}/target/release/doo_runtime.dll",
        target_dir
    );
    println!(
        "cargo:rerun-if-changed={}/target/release/doo_db.dll",
        target_dir
    );
    println!(
        "cargo:rerun-if-changed={}/target/release/doo_auth.dll",
        target_dir
    );
}
