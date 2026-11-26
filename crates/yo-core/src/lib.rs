//! yo's logic: config, the ledger, sessions, harnesses and runners. Synchronous.
//!
//! The core never prints: output is the caller's job.
//! Every outside effect (commands, the clock, randomness, the environment)
//! goes through [`sys`]; `clippy.toml` in this crate bans the direct calls
//! everywhere else. See `docs/design/core.md`.

#![deny(clippy::print_stdout, clippy::print_stderr, clippy::dbg_macro)]

pub mod sys;
