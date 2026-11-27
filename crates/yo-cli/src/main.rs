//! The yo command line, thin over yo-core.

use std::sync::Arc;

use clap::Parser;
use yo_core::sys::RealSys;

mod cli;
mod ledger_cmds;
mod render;
mod run;
mod setup;
mod teardown;

#[cfg(test)]
mod tests;

fn main() {
    let cli = cli::Cli::parse();
    if let Some(cli::Command::SessionHook { harness }) = &cli.command {
        // The payload is on stdin. Never fails: a ledger hiccup must not get
        // in the way of a session starting or ending.
        let mut payload = String::new();
        let _ = std::io::Read::read_to_string(
            &mut std::io::Read::take(std::io::stdin().lock(), 1 << 20),
            &mut payload,
        );
        run::session_hook(Arc::new(RealSys), harness.as_deref(), &payload);
        return;
    }
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
