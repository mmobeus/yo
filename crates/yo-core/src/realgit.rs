//! Tests against real git: the fake `Sys` with real commands, so git
//! itself says what's dirty and what it refuses to remove. Test-only.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::fake::FakeSys;
use crate::plan::{Action, Note, Report};
use crate::runner::{LaunchSpec, Launched};
use crate::sys::{CommandRunner, CommandSpec, ProcessTable, RealSys, Sys};

pub(crate) static REAL: RealSys = RealSys;

/// The fake, with real commands.
pub(crate) struct GitSys(pub(crate) FakeSys);

impl Sys for GitSys {
    fn now(&self) -> SystemTime {
        self.0.now()
    }
    fn new_session_id(&self) -> String {
        self.0.new_session_id()
    }
    fn pick(&self, n: usize) -> usize {
        self.0.pick(n)
    }
    fn env(&self, key: &str) -> Option<String> {
        self.0.env(key)
    }
    fn home(&self) -> PathBuf {
        self.0.home()
    }
    fn cmd(&self) -> &dyn CommandRunner {
        &REAL
    }
    fn procs(&self) -> &dyn ProcessTable {
        self.0.procs()
    }
    fn hostname(&self) -> String {
        self.0.hostname()
    }
    fn exe(&self) -> PathBuf {
        self.0.exe()
    }
    fn sleep(&self, d: Duration) {
        self.0.sleep(d);
    }
}

/// `git -C dir args...`, for real, with a fixed identity; panics if it fails.
pub(crate) fn run(dir: &Path, args: &[&str]) -> String {
    let spec = CommandSpec::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "init.defaultBranch=main",
        ])
        .arg("-C")
        .arg(dir.to_string_lossy())
        .args(args.iter().copied());
    let out = REAL.run(&spec).unwrap();
    assert!(
        out.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.text()
}

#[derive(Default)]
pub(crate) struct Said(pub(crate) Vec<String>);
impl Report for Said {
    fn note(&mut self, n: &Note) {
        let (Note::Info(s) | Note::Detail(s) | Note::Warn(s)) = n;
        self.0.push(s.clone());
    }
    fn doing(&mut self, _: &Action) {}
    fn launched(&mut self, _: &LaunchSpec, _: &Launched) {}
}
