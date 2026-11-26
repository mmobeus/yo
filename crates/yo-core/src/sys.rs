//! `Sys`: the one place yo-core reaches outside itself (the clock, ids,
//! randomness, the environment, the process table, running commands).
//! The real implementation lives here; tests use fakes.
//!
//! The methods `clippy.toml` bans in the rest of yo-core are allowed here.

#![allow(clippy::disallowed_methods)]
