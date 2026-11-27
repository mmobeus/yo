//! Harnesses: the coding agents' CLIs yo runs (`claude`, `codex`). What's
//! specific to one lives in its module; the core asks through [`Harness`].

use std::path::{Path, PathBuf};

use crate::ledger::HookEvent;
use crate::paths::Paths;
use crate::plan::{Action, Note};
use crate::registry::{Registry, Repo};
use crate::sys::Sys;
use crate::{Ctx, Result};

pub mod claude;
pub mod codex;

/// The harness a ledger record without one ran on: every record before
/// harnesses were recorded.
pub const DEFAULT: &str = claude::NAME;

pub trait Harness: Send + Sync {
    /// `"claude"`, `"codex"`: what a ledger record's `harness` says.
    fn name(&self) -> &str;
    /// Its command when nothing configures one, as shell words.
    fn default_command(&self) -> &str;
    /// Its command for a session of `repo` under profile `picked`, as
    /// configured (`harnesses.<name>`).
    fn command(&self, reg: &Registry, _repo: Option<&Repo>, _picked: Option<&str>) -> String {
        reg.command(self.name(), None, self.default_command())
    }
    /// Who picks a new session's id.
    fn ids(&self) -> IdPolicy;
    /// For a harness that reports its ids: the id of the session yo
    /// launched (`l`), if the harness has recorded it, other than those
    /// `taken`.
    fn reported_id(&self, _sys: &dyn Sys, _l: &Launch<'_>, _taken: &[String]) -> Option<String> {
        None
    }
    /// What a session asked to report back (`--from`) to session `to` is
    /// told: how it can reach `to` from here. By default it runs `yo TO
    /// 'REPORT'`, which types the report into `to`'s tab.
    fn report_back(&self, to: &str) -> String {
        format!(
            "When you finish, or get blocked on something only a person can decide, report back to the session named \"{to}\" by running `yo {} '<report>'`, which types the report into that session: a few lines on the outcome and where the work is (branch, PR, files). Do the work in this session; don't hand it back.",
            shell_word(to)
        )
    }
    /// How else to reach one of its running sessions when yo can't find
    /// its tab (`remote_control`: the session's Remote Control name), as
    /// it follows "reach it ". `None` (the default): there's no other way,
    /// short of ending it and resuming it.
    fn reach_hint(&self, _remote_control: Option<&str>) -> Option<String> {
        None
    }
    /// Its sessions that are running now.
    fn live(&self, sys: &dyn Sys) -> Vec<LiveSession>;
    fn capabilities(&self) -> Capabilities;
    /// The command that starts its TUI for a new session.
    fn tui_command(&self, s: &SessionSpec) -> Argv;
    /// The command that reopens session `id` in its TUI, with
    /// `s.first_message` (if any) as its next prompt, as it is.
    fn resume_command(&self, id: &str, s: &SessionSpec) -> Argv;
    /// Where session `id` ran, from the harness's own records (running or
    /// not): what resuming it needs. `None` when it has no such record.
    fn session_cwd(&self, _sys: &dyn Sys, _id: &str) -> Option<PathBuf> {
        None
    }
    /// Its own setup for a session, before launch: for Claude, the settings
    /// layer, memory and trust. Nothing, for a harness with none.
    fn prepare(&self, _ctx: &Ctx, _p: &PrepareAsk<'_>) -> Result<Prepared> {
        Ok(Prepared::default())
    }
    /// The id of its session yo is running inside (yo run by one of its
    /// tools), if any.
    fn own_session(&self, _sys: &dyn Sys) -> Option<String> {
        None
    }
    /// Whether yo is running inside one of its sessions, id or not.
    fn inside(&self, sys: &dyn Sys) -> bool {
        self.own_session(sys).is_some()
    }
    /// Where its personas (yo's agents) are defined, relative to a
    /// checkout and to `HOME` alike (Claude: `.claude/agents`). `None`
    /// without `personas`.
    fn persona_dir(&self) -> Option<&'static str> {
        None
    }
    /// Its ledger hook's payload as a start or an end, for a harness with
    /// `ledger_hooks`. `None` for anything it doesn't recognize.
    fn hook_event(&self, _payload: &str) -> Option<HookEvent> {
        None
    }
    /// What `yo setup` installs for it, from copies built into yo: files
    /// (skills, a settings layer), and the settings `--merge-settings`
    /// merges into its own. Nothing, for a harness with none.
    fn bundle(&self, _sys: &dyn Sys, _paths: &Paths) -> Bundle {
        Bundle::default()
    }
    /// The ledger hook `yo setup` can add to the harness's own hooks file,
    /// for a harness whose hooks can't come in per session (Codex). It
    /// runs for every session of the harness, yo's or not, so it's only
    /// added when asked. `None` for a harness with no such file.
    fn global_hook(&self, _sys: &dyn Sys) -> Option<GlobalHook> {
        None
    }
}

/// A harness's hooks file, and yo's ledger hook in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlobalHook {
    pub path: PathBuf,
    /// The hook's command for this yo.
    pub command: String,
    /// The command of yo's hook in the file now, if it has one.
    pub installed: Option<String>,
    /// Whether the harness has approved the hook that's there. Always true
    /// for a harness that doesn't ask.
    pub approved: bool,
    /// The file with yo's hook running `command`, or without it. `None`
    /// when the file isn't shaped like a hooks file (or, for `without`,
    /// has no hook of yo's).
    pub with: Option<String>,
    pub without: Option<String>,
    /// What the hook is for (a sentence or two), and how it's approved, for
    /// the person.
    pub offer: &'static str,
    pub approve: &'static str,
}

/// A session yo launched, as a harness that reports its ids is asked for
/// the one it started.
#[derive(Clone, Copy, Debug)]
pub struct Launch<'a> {
    pub dir: &'a Path,
    /// When yo launched it, in unix seconds.
    pub since: i64,
    /// The codename yo told it, which only this launch has been given.
    pub codename: &'a str,
}

/// Who picks a new session's id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdPolicy {
    /// yo does, and passes it at launch (Claude: `--session-id`), so the
    /// ledger has it from the start.
    Picked,
    /// The harness does, and yo learns it after launch.
    Reported,
}

/// A harness's part of `yo setup`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bundle {
    pub items: Vec<Bundled>,
    pub global: Option<GlobalSettings>,
}

/// A file or directory yo installs, built into the binary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bundled {
    /// Where it lives in a yo checkout (`claude/skills/yo-down`): what
    /// `--link` links to, and how a link an older install made is known.
    pub rel: &'static str,
    pub dest: PathBuf,
    pub kind: BundledKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BundledKind {
    File {
        text: &'static str,
        exec: bool,
    },
    /// A directory of files: `(name, text)`.
    Dir(Vec<(&'static str, &'static str)>),
}

/// Settings merged into the harness's own settings file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlobalSettings {
    pub path: PathBuf,
    /// JSON, as an object.
    pub layer: &'static str,
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
    /// A Remote Control name (an agent's home), which the session takes as
    /// its name too.
    pub remote_control: Option<String>,
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
    Codex(codex::Prep),
}

impl Prep {
    pub fn run(&self, sys: &dyn Sys) -> Result<()> {
        match self {
            Prep::Claude(p) => p.run(sys),
            Prep::Codex(p) => p.run(sys),
        }
    }

    /// The step, for a dry run.
    pub fn planned(&self) -> &'static str {
        match self {
            Prep::Claude(p) => p.planned(),
            Prep::Codex(p) => p.planned(),
        }
    }

    /// The step, as it runs.
    pub fn doing(&self) -> &'static str {
        match self {
            Prep::Claude(p) => p.doing(),
            Prep::Codex(p) => p.doing(),
        }
    }

    /// What its failure means, for a warning: a failed step doesn't stop
    /// the session, the harness just asks.
    pub fn failed(&self) -> &'static str {
        match self {
            Prep::Claude(p) => p.failed(),
            Prep::Codex(p) => p.failed(),
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

/// Whether the person can be asked a question: at a terminal, and outside
/// a harness's session (whose terminal isn't the person's).
pub fn can_ask(sys: &dyn Sys) -> bool {
    sys.interactive() && !all().iter().any(|h| h.inside(sys))
}

/// Every harness yo has.
pub fn all() -> Vec<Box<dyn Harness>> {
    vec![Box::new(claude::Claude), Box::new(codex::Codex)]
}

/// The program each harness runs (the first word of its default command):
/// the job names iTerm2's triggers watch for, to put its tab in yo's
/// workgroup.
pub fn programs() -> Vec<String> {
    all()
        .iter()
        .filter_map(|h| h.default_command().split_whitespace().next())
        .map(str::to_string)
        .collect()
}

/// The harness named `name`, among `all`.
pub fn find<'a>(all: &'a [Box<dyn Harness>], name: &str) -> Result<&'a dyn Harness> {
    all.iter()
        .find(|h| h.name() == name)
        .map(|h| &**h)
        .ok_or_else(|| {
            let names: Vec<&str> = all.iter().map(|h| h.name()).collect();
            crate::Error::Refused(format!(
                "no harness '{name}' (yo has: {})",
                names.join(", ")
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::{CommandSpec, RealSys, Sys};

    #[test]
    fn programs_are_each_harness_s_first_word() {
        assert_eq!(programs(), ["claude", "codex"]);
    }

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
