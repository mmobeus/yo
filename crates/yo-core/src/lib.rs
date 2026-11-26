//! yo's logic: config, the ledger, sessions, harnesses and runners. Synchronous.
//!
//! The core never prints: output is the caller's job.
//! Every outside effect (commands, the clock, randomness, the environment)
//! goes through [`sys`]; `clippy.toml` in this crate bans the direct calls
//! everywhere else. See `docs/design/core.md`.

#![deny(clippy::print_stdout, clippy::print_stderr, clippy::dbg_macro)]

use std::fs;
use std::path::Path;
use std::sync::Arc;

pub mod config;
pub mod error;
#[cfg(any(test, feature = "fake"))]
pub mod fake;
pub mod git;
pub mod github;
pub mod harness;
pub mod ledger;
pub mod machine;
pub mod paths;
pub mod registry;
pub mod runner;
pub mod sys;

pub use error::{Error, Result};

use harness::{Harness, LiveSession};
use ledger::{Ledger, SyncClone};
use paths::Paths;
use registry::Registry;
use sys::Sys;

/// What every command runs with.
pub struct Ctx {
    pub sys: Arc<dyn Sys>,
    pub paths: Paths,
    pub registry: Registry,
    pub harnesses: Vec<Box<dyn Harness>>,
}

impl Ctx {
    /// Loads softly: no config is single-repo mode against `cwd`, and no
    /// repos at all is fine (commands that need one say so themselves).
    pub fn load(sys: Arc<dyn Sys>, cwd: &Path) -> Result<Ctx> {
        let paths = Paths::new(&*sys);
        let registry = Registry::load(&*sys, &paths.config, cwd)?;
        Ok(Ctx {
            sys,
            paths,
            registry,
            harnesses: harness::all(),
        })
    }

    pub fn ledger(&self) -> Ledger {
        Ledger::new(self.sys.clone(), self.paths.clone())
    }

    /// Every harness's live sessions.
    pub fn live(&self) -> Vec<LiveSession> {
        self.harnesses
            .iter()
            .flat_map(|h| h.live(&*self.sys))
            .collect()
    }

    /// The registered clones, for [`Ledger::sync`].
    pub fn sync_clones(&self) -> Vec<SyncClone> {
        self.registry
            .repos()
            .iter()
            .map(|r| SyncClone {
                path: r.path.clone(),
                repo: r.key.clone(),
                title_prefix: self.registry.title_prefix(r),
                profile: r.profile.clone(),
            })
            .collect()
    }
}

/// Writes a file through a temporary file and a rename, so a reader never
/// sees half of it. Makes the directory first.
pub(crate) fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    fs::write(&tmp, data).map_err(|e| Error::io(path, e))?;
    fs::rename(&tmp, path).map_err(|e| Error::io(path, e))
}
