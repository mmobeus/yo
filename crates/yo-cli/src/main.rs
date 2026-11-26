//! The yo command line, thin over yo-core.

use std::sync::Arc;

use clap::Parser;
use yo_core::sys::RealSys;

mod cli;
mod render;
mod run;

#[cfg(test)]
mod tests;

fn main() {
    let cli = cli::Cli::parse();
    let cwd = std::env::current_dir().unwrap_or_else(|_| "/".into());
    let code = run::run(
        &cli,
        Arc::new(RealSys),
        &cwd,
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    );
    std::process::exit(code);
}
