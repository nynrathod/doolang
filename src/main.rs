mod cli;

use clap::Parser;
use cli::{run_cli, Cli};

fn main() {
    // Enable UTF-8 console output on Windows
    #[cfg(target_os = "windows")]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn SetConsoleOutputCP(code_page: u32) -> i32;
        }
        unsafe {
            SetConsoleOutputCP(65001); // 65001 is UTF-8
        }
    }
    
    let cli = Cli::parse();
    
    // Always delegate to CLI logic
    // If no subcommand is provided, run_cli handles showing help/info
    let exit_code = run_cli(cli);
    std::process::exit(exit_code);
}
