//! Harnesses: the coding agents' CLIs yo runs (`claude`, later `codex`).
//! What's specific to one lives in its module; the core asks through
//! [`Harness`].
//!
//! The trait holds what's ported so far: launching (`tui_command`) came
//! with create. Resuming, preparing a session and the rest of the design's
//! trait arrive with the commands that use them.

use std::path::{Path, PathBuf};

use crate::ledger::HookEvent;
use crate::plan::{Action, Note};
use crate::registry::Repo;
use crate::sys::Sys;
use crate::{Ctx, Result};

pub mod claude;

pub trait Harness: Send + Sync {
    /// `"claude"`, `"codex"`: what a ledger record's `harness` says.
    fn name(&self) -> &str;
    /// Its sessions that are running now.
    fn live(&self, sys: &dyn Sys) -> Vec<LiveSession>;
    fn capabilities(&self) -> Capabilities;
    /// The command that starts its TUI for a new session.
    fn tui_command(&self, s: &SessionSpec) -> Argv;
    /// Its own setup for a session, before launch: for Claude, the settings
    /// layer, memory and trust. Nothing, for a harness with none.
    fn prepare(&self, _ctx: &Ctx, _p: &PrepareAsk<'_>) -> Result<Prepared> {
        Ok(Prepared::default())
    }
    /// Its ledger hook's payload as a start or an end, for a harness with
    /// `ledger_hooks`. `None` for anything it doesn't recognize.
    fn hook_event(&self, _payload: &str) -> Option<HookEvent> {
        None
    }
}

/// What a harness has beyond the required parts. Anything it lacks shows
/// up as "not available", never as a failure.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// A settings layer composed per session (Claude: `--settings`).
    pub settings_layer: bool,
    /// Shared memory (Claude: `autoMemoryDirectory`).
    pub memory: bool,
    /// Personas: yo's `agents` (Claude: `--agent`).
    pub personas: bool,
    /// Hooks that keep the ledger current (Claude: `SessionStart`,
    /// `SessionEnd` calling `yo _session-hook`).
    pub ledger_hooks: bool,
    pub remote_control: bool,
    /// `--model`
    pub models: bool,
}

/// What a harness needs to start a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionSpec {
    /// The harness's command, as configured: shell words, not one program
    /// (`claude --permission-mode auto`).
    pub command: String,
    pub dir: PathBuf,
    /// yo's name for the session, passed on so others can address it.
    pub name: String,
    /// The session id, when the harness lets yo pick it.
    pub id: Option<String>,
    pub model: Option<String>,
    pub persona: Option<String>,
    /// "Plucky Gary": yo picks it; the harness decides how the session is told.
    pub codename: String,
    /// What the session is asked to do, without the codename.
    pub first_message: Option<String>,
    /// A settings layer [`Harness::prepare`] composed for it.
    pub settings: Option<PathBuf>,
}

/// What a harness's setup is for.
pub struct PrepareAsk<'a> {
    pub repo: Option<&'a Repo>,
    /// The profile picked (`--profile`, `yo <profile>`).
    pub picked: Option<&'a str>,
    /// What the session is filed under: its repo, profile or directory.
    pub label: &'a str,
    pub dir: &'a Path,
    pub mode: crate::registry::Mode,
    /// The main clone of a worktree yo just made from a trusted repo: its
    /// per-folder state is copied over. `None` in place, or for a repo
    /// yo doesn't pre-trust.
    pub trusted_clone: Option<&'a Path>,
}

/// A harness's setup: steps to run, what to say, and a settings layer for
/// the launch.
#[derive(Debug, Default)]
pub struct Prepared {
    pub notes: Vec<Note>,
    pub actions: Vec<Action>,
    pub settings: Option<PathBuf>,
}

/// A step only one harness knows how to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Prep {
    Claude(claude::Prep),
}

impl Prep {
    pub fn run(&self, sys: &dyn Sys) -> Result<()> {
        match self {
            Prep::Claude(p) => p.run(sys),
        }
    }

    /// What its failure means, for a warning: a failed step doesn't stop
    /// the session, the harness just asks.
    pub fn failed(&self) -> &'static str {
        match self {
            Prep::Claude(p) => p.failed(),
        }
    }
}

/// A command to run in a terminal: variables, the configured command (shell
/// words, used as they are), then arguments (each quoted when it's written
/// out when it needs it).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Argv {
    pub env: Vec<(String, String)>,
    pub command: String,
    pub args: Vec<String>,
}

impl Argv {
    /// The single shell line a terminal runs: `K='v' command 'arg' ...`.
    pub fn line(&self) -> String {
        let mut words: Vec<String> = self
            .env
            .iter()
            .map(|(k, v)| format!("{k}={}", single_quote(v)))
            .collect();
        words.push(self.command.clone());
        words.extend(self.args.iter().map(|a| shell_word(a)));
        words.join(" ")
    }
}

/// `s` as one shell word: as it is when it's only characters no shell
/// treats specially, else [`single_quote`]d.
pub fn shell_word(s: &str) -> String {
    let plain = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,".contains(c));
    if plain {
        s.to_string()
    } else {
        single_quote(s)
    }
}

/// `s` in single quotes, with each `'` as `'\''`: safe on a shell line and
/// still readable there (unlike `printf %q`'s backslashes).
pub fn single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// A session a harness says is running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveSession {
    /// The harness's name for itself.
    pub harness: String,
    /// The harness's session id.
    pub id: String,
    pub pid: u32,
    pub cwd: PathBuf,
    /// The harness's name for the session, if it has one.
    pub name: Option<String>,
    /// When the harness last updated it, in its own units (Claude: unix ms).
    /// Only compared with others from the same harness.
    pub updated_at: i64,
}

/// Whether `id` looks like a session id yo can pass on: letters, digits,
/// `-` and `_`, at most 128 of them. It ends up on command lines
/// (`--resume <id>`), so anything else is no id at all.
pub fn valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Every harness yo has.
pub fn all() -> Vec<Box<dyn Harness>> {
    vec![Box::new(claude::Claude)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::{CommandSpec, RealSys, Sys};

    #[test]
    fn quoting_survives_the_shell() {
        // Through a real `sh`: what goes in comes back out.
        for s in [
            "",
            "plain",
            "it's",
            "'",
            "''",
            "a b\tc",
            "$HOME `id` $(id) \\ \" ! * ? ~ #",
            "line\nbreak",
            "é ✓",
            "--flag=v",
            "~",
            "a,b",
        ] {
            for word in [single_quote(s), shell_word(s)] {
                let spec = CommandSpec::new("sh")
                    .arg("-c")
                    .arg(format!("printf %s {word}"));
                let out = RealSys.cmd().run(&spec).unwrap();
                assert_eq!(String::from_utf8(out.stdout).unwrap(), s, "{s:?}");
            }
        }
    }

    #[test]
    fn lines() {
        let a = Argv {
            env: vec![("K".into(), "a b".into())],
            command: "claude --permission-mode auto".into(),
            args: vec!["--session-id".into(), "x".into(), "it's".into()],
        };
        assert_eq!(
            a.line(),
            "K='a b' claude --permission-mode auto --session-id x 'it'\\''s'"
        );
    }
}
