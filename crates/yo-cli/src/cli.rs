//! The command line, as clap parses it. Only syntax: what needs the config
//! (whether `--repo` names a repo, what `yo NAME` means) is checked when the
//! command runs.

use clap::{Args, Parser, Subcommand};

/// Start, find and tear down coding sessions, each on its own git worktree.
#[derive(Debug, Parser)]
#[command(
    name = "yo",
    disable_help_subcommand = true,
    args_conflicts_with_subcommands = true,
    after_help = "`yo help` prints the full guide, with the repos yo can see."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
    #[command(flatten)]
    pub up: Up,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// The full guide, and the repos, profiles and agents yo can see.
    Help,
    /// Show or change the config.
    Config {
        #[command(subcommand)]
        command: Option<ConfigCmd>,
    },
    /// The sessions in the ledger.
    #[command(visible_alias = "ls")]
    Sessions,
    /// Reopen a session (no query: list them).
    Resume(Resume),
    /// Reopen every resumable session.
    ResumeAll(ResumeAll),
    /// Tear down the current session.
    #[command(visible_aliases = ["rm", "teardown"])]
    Down(Down),
    /// Remove worktrees whose work has landed.
    Sweep(Sweep),
    /// Record the current session in the ledger.
    Adopt(Adopt),
    /// Retitle the current session (no title: resync titles).
    Title { title: Option<String> },
    /// Drop a session from the ledger.
    Forget { query: String },
    /// List agents, or start or reach one's home.
    Agent(Agent),
    /// The ledger hook, run by the harness with its payload on stdin.
    #[command(name = "_session-hook", hide = true)]
    SessionHook,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCmd {
    /// The config file and what it registers (the default).
    Show,
    /// The config file's path.
    Path,
    /// Edit the config in $EDITOR, then check it.
    Edit,
    /// Register a repo: a clone, a GitHub URL, or owner/repo.
    AddRepo {
        target: String,
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        key: Option<String>,
    },
}

/// Where a session's tab goes.
#[derive(Debug, Default, Args)]
pub struct Window {
    /// A new iTerm window, not a tab in the shared one.
    #[arg(long, visible_alias = "window", overrides_with = "tab")]
    pub new_window: bool,
    /// A tab in the shared window.
    #[arg(long, overrides_with = "new_window")]
    pub tab: bool,
}

/// `yo [flags] [REPO|PROFILE] [INPUT] [EXTRA]`: start a session, or reach
/// the one NAME names.
#[derive(Debug, Default, Args)]
pub struct Up {
    /// The repo to work in.
    #[arg(long, value_name = "R")]
    pub repo: Option<String>,
    /// Run under profile P.
    #[arg(long, value_name = "P")]
    pub profile: Option<String>,
    /// No worktree: run in an existing directory.
    #[arg(long, visible_alias = "in-place", conflicts_with = "worktree")]
    pub here: bool,
    /// A worktree, over a `here` default.
    #[arg(long)]
    pub worktree: bool,
    #[command(flatten)]
    pub window: Window,
    /// Where an unknown repo from a URL goes: a profile, or guest.
    #[arg(long, value_name = "PROFILE|guest", value_parser = nonempty)]
    pub add_to: Option<String>,
    /// The tab's title.
    #[arg(long, value_name = "T", value_parser = nonempty)]
    pub title: Option<String>,
    /// The session to report back to.
    #[arg(long, value_name = "SESSION", value_parser = nonempty)]
    pub from: Option<String>,
    /// The model for this session.
    #[arg(long, value_name = "M", value_parser = model)]
    pub model: Option<String>,
    /// Run the session as agent NAME.
    #[arg(long, value_name = "NAME", value_parser = agent_name)]
    pub agent: Option<String>,
    /// Say what would happen, and change nothing.
    #[arg(long, short = 'n')]
    pub dry_run: bool,
    /// `[REPO|PROFILE] [INPUT] [EXTRA]`: INPUT is a name, a branch, a PR or
    /// issue URL, or `-` for an unnamed session; EXTRA is a first message.
    #[arg(value_name = "ARGS", num_args = 0..=3)]
    pub args: Vec<String>,
}

#[derive(Debug, Args)]
pub struct Resume {
    /// A name, session id (or 8+ character prefix), or directory.
    pub query: Option<String>,
    #[arg(long, value_name = "M", value_parser = model)]
    pub model: Option<String>,
    #[command(flatten)]
    pub window: Window,
}

#[derive(Debug, Args)]
pub struct ResumeAll {
    /// Say what would be resumed, and resume nothing.
    #[arg(long, short = 'n')]
    pub dry_run: bool,
    #[command(flatten)]
    pub window: Window,
}

#[derive(Debug, Args)]
pub struct Down {
    /// Say what would happen, and change nothing.
    #[arg(long, short = 'n')]
    pub dry_run: bool,
    /// Don't ask (refuses to lose uncommitted work without --discard).
    #[arg(long, short = 'y')]
    pub yes: bool,
    /// With --yes: throw away uncommitted work.
    #[arg(long, requires = "yes")]
    pub discard: bool,
}

#[derive(Debug, Args)]
pub struct Sweep {
    /// Remove what's listed (default: only list).
    #[arg(long)]
    pub apply: bool,
    /// Delete the swept worktrees' branches too.
    #[arg(long)]
    pub delete_branches: bool,
    #[arg(long)]
    pub include_stale: bool,
    #[arg(long)]
    pub include_suspended: bool,
    /// Only this repo (repeatable).
    #[arg(long = "repo", value_name = "R")]
    pub repos: Vec<String>,
}

#[derive(Debug, Args)]
pub struct Adopt {
    /// The session's id (default: this session's, from the harness).
    #[arg(long, value_name = "ID")]
    pub session_id: Option<String>,
    #[arg(long, value_name = "T")]
    pub title: Option<String>,
}

#[derive(Debug, Args)]
pub struct Agent {
    pub name: Option<String>,
    /// A message for its home.
    pub message: Option<String>,
    /// Say what would happen, and change nothing.
    #[arg(long, short = 'n')]
    pub dry_run: bool,
    #[arg(long, value_name = "M", value_parser = model)]
    pub model: Option<String>,
    #[command(flatten)]
    pub window: Window,
}

fn nonempty(v: &str) -> Result<String, String> {
    if v.is_empty() {
        Err("needs a value".to_string())
    } else {
        Ok(v.to_string())
    }
}

/// A model alias or id. It ends up on a command line, so only the
/// characters model names use.
fn model(v: &str) -> Result<String, String> {
    let ok = !v.is_empty()
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._:[]-".contains(c));
    if ok {
        Ok(v.to_string())
    } else {
        Err("expected an alias like opus, sonnet[1m] or haiku, or a full model id".to_string())
    }
}

fn agent_name(v: &str) -> Result<String, String> {
    if yo_core::registry::valid_key(v) {
        Ok(v.to_string())
    } else {
        Err("names may only contain letters, digits, . _ -".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("yo").chain(args.iter().copied()))
    }

    #[test]
    fn definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn subcommands_and_aliases() {
        assert!(matches!(
            parse(&["sessions"]).unwrap().command,
            Some(Command::Sessions)
        ));
        assert!(matches!(
            parse(&["ls"]).unwrap().command,
            Some(Command::Sessions)
        ));
        assert!(matches!(
            parse(&["rm"]).unwrap().command,
            Some(Command::Down(_))
        ));
        assert!(matches!(
            parse(&["config"]).unwrap().command,
            Some(Command::Config { command: None })
        ));
        assert!(matches!(
            parse(&["config", "path"]).unwrap().command,
            Some(Command::Config {
                command: Some(ConfigCmd::Path)
            })
        ));
        assert!(parse(&[]).unwrap().command.is_none());
    }

    #[test]
    fn anything_else_is_up_with_flags_anywhere() {
        let c = parse(&["--here", "api", "--title=T", "name", "--tab", "do it"]).unwrap();
        assert!(c.command.is_none());
        assert!(c.up.here);
        assert_eq!(c.up.title.as_deref(), Some("T"));
        assert!(c.up.window.tab && !c.up.window.new_window);
        let w = parse(&["--tab", "--new-window"]).unwrap();
        assert!(w.up.window.new_window && !w.up.window.tab);
        assert_eq!(c.up.args, ["api", "name", "do it"]);
        let c = parse(&["-", "prod is broken"]).unwrap();
        assert_eq!(c.up.args, ["-", "prod is broken"]);
        let c = parse(&["--repo", "api", "x"]).unwrap();
        assert_eq!(c.up.repo.as_deref(), Some("api"));
    }

    #[test]
    fn values_that_reach_command_lines_are_checked() {
        assert!(parse(&["--model=opus[1m]"]).is_ok());
        assert!(parse(&["--model=opus;rm"]).is_err());
        assert!(parse(&["--model="]).is_err());
        assert!(parse(&["--agent=a b"]).is_err());
        assert!(parse(&["--title="]).is_err());
        assert!(parse(&["resume", "x", "--model=a b"]).is_err());
    }

    #[test]
    fn usage_errors() {
        assert!(parse(&["--nope"]).is_err());
        assert!(parse(&["--here", "--worktree"]).is_err());
        assert!(parse(&["a", "b", "c", "d"]).is_err());
        assert!(parse(&["forget"]).is_err());
        assert!(parse(&["down", "--discard"]).is_err());
        assert!(parse(&["sweep", "--nope"]).is_err());
        let s = parse(&["sweep", "--repo", "a", "--repo=b"]).unwrap();
        let Some(Command::Sweep(s)) = s.command else {
            panic!()
        };
        assert_eq!(s.repos, ["a", "b"]);
    }
}
