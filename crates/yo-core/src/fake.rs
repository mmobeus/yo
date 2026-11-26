//! A fake [`Sys`] for tests: a settable clock, queued ids and picks, a
//! private environment, canned command answers with every call recorded, and
//! a process table. Built for yo-core's own tests, and for other crates'
//! tests with the `fake` feature (as a dev-dependency only).
//!
//! It makes no outside call of its own. [`TempDir`] gives a test a
//! throwaway directory to use as `HOME` and the state directory.

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::sys::{CommandRunner, CommandSpec, Output, ProcessTable, Sys};

/// A fake system. Everything starts empty: no variables, no processes, and
/// every command fails (exit 127) until it's given an answer.
pub struct FakeSys {
    now: Mutex<SystemTime>,
    ids: Mutex<VecDeque<String>>,
    picks: Mutex<VecDeque<usize>>,
    env: Mutex<BTreeMap<String, String>>,
    home: PathBuf,
    hostname: String,
    commands: FakeCommands,
    procs: FakeProcs,
}

impl FakeSys {
    /// A fake whose `HOME` is `home`, at 2026-11-26 08:54:22 UTC.
    pub fn new(home: impl Into<PathBuf>) -> Self {
        FakeSys {
            now: Mutex::new(UNIX_EPOCH + Duration::from_secs(1_795_683_262)),
            ids: Mutex::new(VecDeque::new()),
            picks: Mutex::new(VecDeque::new()),
            env: Mutex::new(BTreeMap::new()),
            home: home.into(),
            hostname: "fakehost".to_string(),
            commands: FakeCommands::default(),
            procs: FakeProcs::default(),
        }
    }

    pub fn set_now(&self, secs: u64) {
        *lock(&self.now) = UNIX_EPOCH + Duration::from_secs(secs);
    }

    /// Queues the next ids `new_session_id` returns.
    pub fn push_ids<I: IntoIterator<Item = S>, S: Into<String>>(&self, ids: I) {
        lock(&self.ids).extend(ids.into_iter().map(Into::into));
    }

    /// Queues the next answers `pick` gives (each taken modulo `n`).
    pub fn push_picks(&self, picks: impl IntoIterator<Item = usize>) {
        lock(&self.picks).extend(picks);
    }

    pub fn set_env(&self, key: &str, value: &str) {
        lock(&self.env).insert(key.to_string(), value.to_string());
    }

    /// Answers a command run with exactly this program and these args.
    pub fn answer(&self, program: &str, args: &[&str], out: Output) {
        lock(&self.commands.answers).push(Answer {
            program: program.to_string(),
            args: args.iter().map(|a| (*a).to_string()).collect(),
            out,
        });
    }

    /// Answers with success and this stdout.
    pub fn answer_ok(&self, program: &str, args: &[&str], stdout: &str) {
        self.answer(program, args, ok(stdout));
    }

    /// Every command run so far, in order.
    pub fn calls(&self) -> Vec<CommandSpec> {
        lock(&self.commands.calls).clone()
    }

    /// A running process, with its command name and tty.
    pub fn add_process(&self, pid: u32, comm: &str, tty: Option<&str>) {
        lock(&self.procs.table).insert(pid, (comm.to_string(), tty.map(str::to_string)));
    }
}

/// A successful command's output.
pub fn ok(stdout: &str) -> Output {
    Output {
        status: Some(0),
        stdout: stdout.as_bytes().to_vec(),
        stderr: Vec::new(),
    }
}

/// A failed command's output.
pub fn failed(code: i32) -> Output {
    Output {
        status: Some(code),
        ..Output::default()
    }
}

impl Sys for FakeSys {
    fn now(&self) -> SystemTime {
        *lock(&self.now)
    }

    fn new_session_id(&self) -> String {
        lock(&self.ids)
            .pop_front()
            .unwrap_or_else(|| "00000000-0000-4000-8000-000000000000".to_string())
    }

    fn pick(&self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        lock(&self.picks).pop_front().unwrap_or(0) % n
    }

    fn env(&self, key: &str) -> Option<String> {
        lock(&self.env).get(key).cloned()
    }

    fn home(&self) -> PathBuf {
        self.home.clone()
    }

    fn cmd(&self) -> &dyn CommandRunner {
        &self.commands
    }

    fn procs(&self) -> &dyn ProcessTable {
        &self.procs
    }

    fn hostname(&self) -> String {
        self.hostname.clone()
    }

    fn exe(&self) -> PathBuf {
        PathBuf::from("/opt/yo/bin/yo")
    }
}

struct Answer {
    program: String,
    args: Vec<String>,
    out: Output,
}

#[derive(Default)]
struct FakeCommands {
    answers: Mutex<Vec<Answer>>,
    calls: Mutex<Vec<CommandSpec>>,
}

impl CommandRunner for FakeCommands {
    fn run(&self, cmd: &CommandSpec) -> io::Result<Output> {
        lock(&self.calls).push(cmd.clone());
        let answers = lock(&self.answers);
        // The latest answer wins, so a test can override an earlier one.
        let hit = answers
            .iter()
            .rev()
            .find(|a| a.program == cmd.program && a.args == cmd.args);
        Ok(hit.map_or_else(
            || Output {
                status: Some(127),
                stdout: Vec::new(),
                stderr: format!("fake: no answer for {} {:?}", cmd.program, cmd.args).into_bytes(),
            },
            |a| a.out.clone(),
        ))
    }
}

#[derive(Default)]
struct FakeProcs {
    table: Mutex<BTreeMap<u32, (String, Option<String>)>>,
}

impl ProcessTable for FakeProcs {
    fn comm(&self, pid: u32) -> Option<String> {
        lock(&self.table).get(&pid).map(|(c, _)| c.clone())
    }

    fn tty(&self, pid: u32) -> Option<String> {
        lock(&self.table).get(&pid).and_then(|(_, t)| t.clone())
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A directory removed when dropped.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new() -> io::Result<Self> {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("yo-test-{}-{n}", std::process::id()));
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        std::fs::create_dir_all(&dir)?;
        // Resolved, so paths compare equal to what canonicalize gives
        // (macOS's /var is a link to /private/var).
        Ok(TempDir(dir.canonicalize()?))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
