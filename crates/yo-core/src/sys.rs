//! `Sys`: the one place yo-core reaches outside itself (the clock, ids,
//! randomness, the environment, the process table, running commands).
//! The real implementation lives here; tests use [`crate::fake`] (built for
//! yo-core's own tests, and for other crates' tests with the `fake`
//! feature). The fake lives outside this module, so it doesn't share the
//! exemption below.
//!
//! The methods `clippy.toml` bans in the rest of yo-core are allowed here.
//! Files are read and written with plain `std::fs`, under paths built from
//! [`Sys::home`] and [`Sys::env`].

#![allow(clippy::disallowed_methods)]

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Everything nondeterministic or outside yo-core.
pub trait Sys: Send + Sync {
    fn now(&self) -> SystemTime;
    /// A new random (v4) UUID, lowercase, as `uuidgen | tr A-Z a-z` gives.
    fn new_session_id(&self) -> String;
    /// A random number in `0..n` (0 when `n` is 0). Codenames; bash: `$RANDOM`.
    fn pick(&self, n: usize) -> usize;
    /// An environment variable. `Some("")` is a variable set to nothing,
    /// which some settings treat differently from an unset one.
    fn env(&self, key: &str) -> Option<String>;
    fn home(&self) -> PathBuf;
    /// git, gh, osascript, claude, ...
    fn cmd(&self) -> &dyn CommandRunner;
    fn procs(&self) -> &dyn ProcessTable;
    fn hostname(&self) -> String;
    /// The yo binary that's running, for hooks that call yo back.
    fn exe(&self) -> PathBuf;
    /// Waits (iTerm loading a profile yo just wrote).
    fn sleep(&self, d: Duration);
}

impl dyn Sys + '_ {
    /// Seconds since the epoch: what the ledger stores, as bash does.
    pub fn unix_now(&self) -> i64 {
        self.now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
    }

    /// An environment variable that's set and not empty (bash's `${X:-...}`).
    pub fn env_nonempty(&self, key: &str) -> Option<String> {
        self.env(key).filter(|v| !v.is_empty())
    }
}

/// Runs a command to completion. A program that can't be started is an
/// `Err`; one that runs and fails is an `Ok` with a failing status.
pub trait CommandRunner: Send + Sync {
    fn run(&self, cmd: &CommandSpec) -> io::Result<Output>;
    /// Starts a command and leaves it running after yo exits, its output
    /// discarded.
    fn spawn(&self, cmd: &CommandSpec) -> io::Result<()>;
}

/// What to run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    pub stdin: Option<Vec<u8>>,
}

impl CommandSpec {
    pub fn new(program: impl Into<String>) -> Self {
        CommandSpec {
            program: program.into(),
            ..CommandSpec::default()
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }
}

/// How a command ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Output {
    /// The exit code; `None` when killed by a signal.
    pub status: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl Output {
    pub fn success(&self) -> bool {
        self.status == Some(0)
    }

    /// stdout as text, with surrounding whitespace trimmed (as `$(...)` does
    /// with trailing newlines).
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).trim().to_string()
    }
}

/// The process table.
pub trait ProcessTable: Send + Sync {
    /// The command name of a running process, or `None` when it isn't
    /// running.
    fn comm(&self, pid: u32) -> Option<String>;
    /// Its controlling terminal (`ttys003`), if it has one.
    fn tty(&self, pid: u32) -> Option<String>;
    /// Its parent's pid, while it's running.
    fn ppid(&self, pid: u32) -> Option<u32>;
    /// Sends it `sig`; `false` when it couldn't (no such process, or not
    /// ours).
    fn signal(&self, pid: u32, sig: Signal) -> bool;
}

/// The signals yo sends: those a closed terminal would.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    /// A hangup: what a terminal's process gets when the terminal closes.
    Hup,
    Term,
}

impl Signal {
    pub fn name(self) -> &'static str {
        match self {
            Signal::Hup => "HUP",
            Signal::Term => "TERM",
        }
    }
}

/// The real thing.
#[derive(Debug, Default)]
pub struct RealSys;

impl Sys for RealSys {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }

    fn new_session_id(&self) -> String {
        let mut b = random_bytes();
        b[6] = (b[6] & 0x0f) | 0x40;
        b[8] = (b[8] & 0x3f) | 0x80;
        let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
        format!(
            "{}-{}-{}-{}-{}",
            &hex[0..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..32]
        )
    }

    fn pick(&self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        let b = random_bytes();
        let mut x = [0u8; 8];
        x.copy_from_slice(&b[..8]);
        let r = u64::from_le_bytes(x);
        usize::try_from(r % n as u64).unwrap_or(0)
    }

    fn env(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }

    fn home(&self) -> PathBuf {
        std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)
            .or_else(std::env::home_dir)
            .unwrap_or_else(|| PathBuf::from("/"))
    }

    fn cmd(&self) -> &dyn CommandRunner {
        self
    }

    fn procs(&self) -> &dyn ProcessTable {
        self
    }

    fn hostname(&self) -> String {
        Command::new("hostname")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| "localhost".to_string())
    }

    fn exe(&self) -> PathBuf {
        RealSys::current_exe()
    }

    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

impl RealSys {
    fn current_exe() -> PathBuf {
        std::env::current_exe().unwrap_or_else(|_| PathBuf::from("yo"))
    }
}

impl CommandRunner for RealSys {
    fn run(&self, spec: &CommandSpec) -> io::Result<Output> {
        let mut cmd = Command::new(&spec.program);
        cmd.args(&spec.args);
        if let Some(dir) = &spec.cwd {
            cmd.current_dir(dir);
        }
        cmd.envs(spec.env.iter().map(|(k, v)| (k, v)));
        cmd.stdin(if spec.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn()?;
        if let (Some(input), Some(mut pipe)) = (&spec.stdin, child.stdin.take()) {
            pipe.write_all(input)?;
        }
        let out = child.wait_with_output()?;
        Ok(Output {
            status: out.status.code(),
            stdout: out.stdout,
            stderr: out.stderr,
        })
    }

    fn spawn(&self, spec: &CommandSpec) -> io::Result<()> {
        let mut cmd = Command::new(&spec.program);
        cmd.args(&spec.args);
        if let Some(dir) = &spec.cwd {
            cmd.current_dir(dir);
        }
        cmd.envs(spec.env.iter().map(|(k, v)| (k, v)));
        cmd.stdin(if spec.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
        // A process group of its own, so the hangup that ends yo's terminal
        // doesn't end it too.
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        let mut child = cmd.spawn()?;
        if let (Some(input), Some(mut pipe)) = (&spec.stdin, child.stdin.take()) {
            pipe.write_all(input)?;
        }
        // Not waited for: it outlives yo, as bash's `( ... ) & disown` did.
        Ok(())
    }
}

impl ProcessTable for RealSys {
    fn comm(&self, pid: u32) -> Option<String> {
        ps_field(pid, "comm=")
    }

    fn tty(&self, pid: u32) -> Option<String> {
        ps_field(pid, "tty=").filter(|t| t != "??" && t != "?")
    }

    fn ppid(&self, pid: u32) -> Option<u32> {
        ps_field(pid, "ppid=").and_then(|p| p.parse().ok())
    }

    fn signal(&self, pid: u32, sig: Signal) -> bool {
        // pid 0 and 1 aren't a session's: kill would read 0 as "my group".
        pid > 1
            && Command::new("kill")
                .args([&format!("-{}", sig.name()), &pid.to_string()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
    }
}

/// One `ps` column for one pid; `None` when there's no such process.
fn ps_field(pid: u32, field: &str) -> Option<String> {
    let out = Command::new("ps")
        .args(["-o", field, "-p", &pid.to_string()])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !text.is_empty()).then_some(text)
}

/// 16 random bytes from the system's generator.
fn random_bytes() -> [u8; 16] {
    let mut b = [0u8; 16];
    // /dev/urandom exists on every system yo runs on; if it somehow can't
    // be read, fall back to the clock rather than fail a codename.
    let read = File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b));
    if read.is_err() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        b = nanos.to_le_bytes();
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids_are_v4_uuids() {
        let id = RealSys.new_session_id();
        assert_eq!(id.len(), 36);
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(
            parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12]
        );
        assert!(parts[2].starts_with('4'));
        assert!(matches!(&parts[3][..1], "8" | "9" | "a" | "b"));
        assert_ne!(id, RealSys.new_session_id());
    }

    #[test]
    fn pick_stays_in_range() {
        for n in [1, 2, 7] {
            assert!(RealSys.pick(n) < n);
        }
        assert_eq!(RealSys.pick(0), 0);
    }
}
