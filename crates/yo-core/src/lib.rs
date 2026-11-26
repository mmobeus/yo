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
pub mod create;
pub mod error;
#[cfg(any(test, feature = "fake"))]
pub mod fake;
pub mod git;
pub mod github;
pub mod harness;
pub mod ledger;
pub mod machine;
pub mod naming;
pub mod ojson;
pub mod paths;
pub mod place;
pub mod plan;
pub mod reach;
pub mod registry;
pub mod resolve;
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

/// Writes a file through a temporary file beside it and a rename, so a
/// reader never sees half of it. Makes the directory first. The temporary
/// name is this process's own, so two writers can't share one, and a file
/// that's there keeps its mode (a config, or `~/.claude.json`, may be
/// private).
pub(crate) fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}.tmp", std::process::id()));
    let tmp = std::path::PathBuf::from(tmp);
    let done = fs::write(&tmp, data)
        .and_then(|()| match fs::metadata(path) {
            Ok(m) => fs::set_permissions(&tmp, m.permissions()),
            Err(_) => Ok(()),
        })
        .and_then(|()| fs::rename(&tmp, path));
    if let Err(e) = done {
        let _ = fs::remove_file(&tmp);
        return Err(Error::io(path, e));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{FakeSys, TempDir};

    #[test]
    fn atomic_writes_keep_the_mode_and_leave_no_temp_file() {
        use std::os::unix::fs::PermissionsExt;
        let t = TempDir::new().unwrap();
        let f = t.path().join("d/f.json");
        write_atomic(&f, b"one").unwrap();
        fs::set_permissions(&f, fs::Permissions::from_mode(0o600)).unwrap();
        write_atomic(&f, b"two").unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(), "two");
        assert_eq!(
            fs::metadata(&f).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(fs::read_dir(t.path().join("d")).unwrap().count(), 1);
    }

    #[test]
    fn sync_clones_carry_prefixes_and_profiles() {
        let t = TempDir::new().unwrap();
        let cfg = t.path().join("config.json");
        fs::write(
            &cfg,
            r#"{"profiles": {"p": {"repos": {"b": {"path": "/b"}}}},
                "repos": {"a": {"path": "/a"}, "c": {"path": "/c", "titlePrefix": "C: "}}}"#,
        )
        .unwrap();
        let sys = FakeSys::new(t.path());
        sys.set_env("YO_CONFIG", &cfg.to_string_lossy());
        let ctx = Ctx::load(Arc::new(sys), t.path()).unwrap();
        let got: Vec<(String, String, Option<String>)> = ctx
            .sync_clones()
            .into_iter()
            .map(|c| (c.repo, c.title_prefix, c.profile))
            .collect();
        assert_eq!(
            got,
            [
                ("a".into(), String::new(), None),
                ("c".into(), "C: ".into(), None),
                ("b".into(), "[b] ".into(), Some("p".into())),
            ]
        );
        assert!(ctx.live().is_empty());
    }
}
