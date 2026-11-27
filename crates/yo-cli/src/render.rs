//! What the read-only commands print, from the core's data. The text is
//! what the bash `yo` printed.

use std::fmt::Write as _;

use yo_core::Ctx;
use yo_core::harness::LiveSession;
use yo_core::ledger::{Entry, Record, State, state_of, waiting};
use yo_core::plan::{Action, BranchSpec, Note, Plan, Report};
use yo_core::reach::tabless;
use yo_core::registry::Mode;
use yo_core::runner::Tabs;
use yo_core::runner::{LaunchSpec, Launched};

/// `yo help`: the guide, then what yo can see.
pub fn help(ctx: &Ctx) -> String {
    let mut s = include_str!("guide.txt").to_string();
    s.push_str(&registry(ctx));
    s
}

/// The repos, profiles and agents, as `yo help` and `yo config show` end.
pub fn registry(ctx: &Ctx) -> String {
    let reg = &ctx.registry;
    let mut s = String::from("configured repos:\n");
    match &reg.config {
        Some(p) => {
            let _ = writeln!(s, "  (from {})", p.display());
        }
        None => {
            s.push_str("  (no config file — single-repo mode, inferred from the current clone)\n")
        }
    }
    let default = reg.default_repo().map(|r| r.key.clone());
    for r in reg.repos() {
        let mut base = reg.base_ref(&*ctx.sys, r);
        if reg.mode(None, Some(r), None) == Mode::Here {
            base.push_str(", in place");
        }
        if let Some(p) = &r.profile {
            let _ = write!(base, ", profile: {p}");
        }
        let mark = if default.as_ref() == Some(&r.key) {
            "*"
        } else {
            " "
        };
        let _ = writeln!(
            s,
            "  {mark} {:<12} {} (base: {base})",
            r.key,
            r.path.display()
        );
    }
    s.push_str("  (* = default)\n");
    if !reg.profiles().is_empty() {
        s.push_str("profiles:\n");
        for p in reg.profiles() {
            let mut d = String::new();
            if let Some(dir) = &p.dir {
                let _ = write!(d, "{}  ", dir.display());
            }
            if !p.owners.is_empty() {
                let _ = write!(d, "owners: {}  ", p.owners.join(" "));
            }
            let repos: Vec<&str> = reg
                .repos_of_profile(&p.name)
                .map(|r| r.key.as_str())
                .collect();
            let repos = if repos.is_empty() {
                "(none)".to_string()
            } else {
                repos.join(" ")
            };
            let _ = writeln!(s, "    {:<12} {d}repos: {repos}", p.name);
        }
    }
    if !reg.agents().is_empty() {
        s.push_str("agents:\n");
        for a in reg.agents() {
            let _ = writeln!(s, "    {:<12} repo: {}", a.name, a.repo);
        }
    }
    s
}

/// `yo config show`.
pub fn config_show(ctx: &Ctx) -> String {
    let path = &ctx.paths.config;
    let mut s = if path.is_file() {
        format!("config: {}\n", path.display())
    } else {
        format!("config: none yet ({})\n", path.display())
    };
    s.push_str(&registry(ctx));
    s
}

/// `yo sessions`: one row per record, then what the states mean.
pub fn sessions(
    ctx: &Ctx,
    entries: &[Entry],
    live: &[LiveSession],
    shown: Option<&Tabs>,
) -> String {
    if entries.is_empty() {
        return format!(
            "no sessions in the ledger ({})\n",
            ctx.ledger().dir().display()
        );
    }
    let now = ctx.sys.unix_now();
    let status = |r: &Record| {
        let state = state_of(r, live);
        if tabless(ctx, r, live, shown).is_some() {
            (state, "running (no tab)")
        } else if waiting(r, state, now) {
            (state, "waiting?")
        } else {
            (state, state.as_str())
        }
    };
    let rows: Vec<(&Entry, State, &str)> = entries
        .iter()
        .map(|e| {
            let (st, word) = status(&e.record);
            (e, st, word)
        })
        .collect();
    let width = rows
        .iter()
        .map(|(_, _, w)| w.len())
        .max()
        .unwrap_or(0)
        .max(8);
    let mut s = format!(
        "{:<width$} {:<7} {:<12} {:<10} {:<36} NAME\n",
        "STATUS", "PID", "REPO", "AGENT", "SESSION"
    );
    for (e, state, word) in &rows {
        let r = &e.record;
        let pid = match state {
            State::Running { pid } => pid.to_string(),
            _ => "-".to_string(),
        };
        let dash = |v: &Option<String>| {
            v.clone()
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| "-".into())
        };
        let mut name = dash(&r.name);
        let title = r
            .title
            .clone()
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| name.clone());
        let mut repo = dash(&r.repo);
        if repo == "-"
            && let Some(p) = r.profile.as_ref().filter(|p| !p.is_empty())
        {
            repo = format!("({p})");
        }
        if title != name {
            name = format!("{name} — {title}");
        }
        if let Some(c) = r.codename.as_ref().filter(|c| !c.is_empty())
            && !name.contains(c.as_str())
        {
            name = format!("{name} ({c})");
        }
        if r.mode_or_default() == Mode::Here {
            name.push_str("  (in place)");
        }
        if r.home && !name.contains("(home)") {
            name.push_str("  (home)");
        }
        let _ = writeln!(
            s,
            "{:<width$} {:<7} {:<12} {:<10} {:<36} {name}",
            word,
            pid,
            repo,
            dash(&r.agent),
            dash(&r.session_id),
        );
    }
    s.push_str(
        "\nready = resumable (yo resume <name|id>, or yo resume-all); ended = quit with /exit\n",
    );
    s.push_str(
        "(yo resume <name> still reopens it; resume-all skips it); gone = directory removed\n",
    );
    if rows.iter().any(|(_, _, w)| *w == "waiting?") {
        s.push_str(
            "waiting? = no session id yet, a while after launch: Codex records a session once its first\n\
             message goes in, so its tab is likely stopped at a prompt (trust, hook review, login)\n",
        );
    }
    if rows.iter().any(|(_, _, w)| *w == "running (no tab)") {
        s.push_str(
            "running (no tab) = still running, but its tab was closed (iTerm can leave a chat running when\n\
             its tab is closed showing CLI); end it with kill -HUP <pid>, then yo resume <name>\n",
        );
    }
    s
}

/// A plan, for `--dry-run`: what was found, then each step.
pub fn dry_run(plan: &Plan) -> String {
    let mut s = String::new();
    for n in &plan.notes {
        note(&mut s, n);
    }
    s.push_str("dry run: would\n");
    for a in &plan.actions {
        let _ = writeln!(s, "  - {}", action(a));
    }
    s.push_str("dry run: nothing changed.\n");
    s
}

fn note(s: &mut String, n: &Note) {
    let _ = match n {
        Note::Info(t) => writeln!(s, "==> {t}"),
        Note::Detail(t) => writeln!(s, "        {t}"),
        Note::Warn(t) => writeln!(s, "yo: {t}"),
    };
}

/// One step, in words.
fn action(a: &Action) -> String {
    match a {
        Action::Fetch { what, .. } => format!("fetch {what}"),
        Action::AddWorktree { path, branch, .. } => {
            format!(
                "create worktree {} on {}",
                path.display(),
                branch_words(branch)
            )
        }
        Action::SetUpstream {
            remote,
            merge,
            branch,
            ..
        } => {
            format!("set {branch} to pull {remote} {merge}")
        }
        Action::MirrorFiles { files, .. } => {
            format!(
                "copy {} from the main clone, if gitignored",
                files.join(" ")
            )
        }
        Action::MakeDir { path, what } => format!("make the {what} directory {}", path.display()),
        Action::WriteFile { path, .. } => format!("write the settings layer {}", path.display()),
        Action::Prepare(p) => p.planned().to_string(),
        Action::Ask { question, prep, .. } => {
            format!("ask: {question}\n      (yes: {})", prep.planned())
        }
        Action::PutRecord(e) => format!("record the session in the ledger as {}", e.key.0),
        Action::DropRecord(k) => format!("forget the ledger record {}", k.0),
        Action::RemoveWorktree { path, force, .. } => format!(
            "remove worktree {}{}",
            path.display(),
            if *force {
                " (forced: losing its changes)"
            } else {
                ""
            }
        ),
        Action::PruneWorktrees { repo } => format!("prune {}'s worktrees", repo.display()),
        Action::SweepWorktree {
            path,
            delete_branch,
            ..
        } => format!(
            "remove worktree {}{}",
            path.display(),
            delete_branch
                .as_deref()
                .map(|b| format!(" and branch {b}"))
                .unwrap_or_default()
        ),
        Action::CloseOwnTab { .. } => "close this tab".to_string(),
        Action::CloseTab { tab } => format!("close its tab {tab}"),
        Action::EndSession {
            harness,
            pid,
            after_close,
            ..
        } => {
            if *after_close {
                format!(
                    "end its {harness} session (pid {pid}) if it's still running once the tab closes"
                )
            } else {
                format!("end its {harness} session (pid {pid}), still running")
            }
        }
        Action::RefreshHome { path, remote, base } => format!(
            "move {} to {remote}/{base}, if it has no uncommitted changes",
            path.display()
        ),
        Action::Focus {
            pid, tty, message, ..
        } => format!(
            "bring forward the tab of pid {pid} ({}){}",
            tty.as_deref().unwrap_or("no terminal"),
            message
                .as_deref()
                .map(|m| format!(", then type: {m}"))
                .unwrap_or_default()
        ),
        Action::Launch(l) => format!(
            "open [{}] in {}\n      {}",
            l.title,
            l.dir.display(),
            l.argv.line()
        ),
    }
}

fn branch_words(b: &BranchSpec) -> String {
    match b {
        BranchSpec::Existing(b) => format!("{b} (REUSING existing local branch, as-is)"),
        BranchSpec::Track { branch, start } => {
            format!("{branch} (new local branch tracking {start})")
        }
        BranchSpec::New { branch, start } => format!("{branch} (new, from {start})"),
        BranchSpec::Detached(start) => format!("no branch (detached at {start})"),
    }
}

/// Progress as a plan runs, in bash's words where it had them.
pub struct Progress<'a> {
    out: &'a mut dyn std::io::Write,
    /// Whether the person can be asked, on the terminal.
    asking: bool,
}

impl<'a> Progress<'a> {
    pub fn new(out: &'a mut dyn std::io::Write) -> Self {
        Progress { out, asking: false }
    }

    /// Asks the plan's questions on the terminal (`/dev/tty`).
    pub fn asking(mut self, yes: bool) -> Self {
        self.asking = yes;
        self
    }

    fn put(&mut self, s: &str) {
        let _ = self.out.write_all(s.as_bytes());
    }

    pub fn finish(&mut self) -> std::io::Result<()> {
        self.out.write_all(b"==> ready.\n")
    }
}

impl Report for Progress<'_> {
    fn note(&mut self, n: &Note) {
        let mut s = String::new();
        note(&mut s, n);
        self.put(&s);
    }

    /// A yes unless the answer is no.
    fn ask(&mut self, question: &str) -> bool {
        self.asking && crate::run::ask_yes(question, true)
    }

    fn doing(&mut self, a: &Action) {
        let s = match a {
            Action::Fetch { what, .. } => format!("==> fetching {what}\n"),
            Action::AddWorktree {
                path,
                branch: BranchSpec::Detached(start),
                ..
            } => format!("==> worktree add --detach {} {start}\n", path.display()),
            Action::AddWorktree { path, branch, .. } => format!(
                "==> creating worktree: {}\n        branch: {}\n",
                path.display(),
                branch_words(branch)
            ),
            Action::SetUpstream { remote, merge, .. } => {
                format!("        tracking {remote} {merge} — git pull refreshes the PR\n")
            }
            Action::MakeDir { path, what } => format!("==> {what}: {}\n", path.display()),
            Action::Prepare(p) => format!("==> {}\n", p.doing()),
            Action::Ask { .. } => String::new(),
            Action::MirrorFiles { .. } | Action::WriteFile { .. } => String::new(),
            Action::PutRecord(e) => {
                let r = &e.record;
                let mut s = String::new();
                let mut line = |k: &str, v: &Option<String>| {
                    if let Some(v) = v {
                        let _ = writeln!(s, "        {k:<10}{v}");
                    }
                };
                line("codename:", &r.codename);
                line("session:", &r.session_id);
                line(
                    "agent:",
                    &r.agent.as_ref().map(|a| match &r.remote_control {
                        Some(rc) => format!("{a} (Remote Control: {rc})"),
                        None => a.clone(),
                    }),
                );
                line("brief:", &r.brief);
                s
            }
            Action::Launch(_)
            | Action::Focus { .. }
            | Action::DropRecord(_)
            | Action::RemoveWorktree { .. }
            | Action::PruneWorktrees { .. }
            | Action::SweepWorktree { .. }
            | Action::CloseOwnTab { .. }
            | Action::CloseTab { .. }
            | Action::EndSession { .. }
            | Action::RefreshHome { .. } => String::new(),
        };
        self.put(&s);
    }

    fn launched(&mut self, spec: &LaunchSpec, l: &Launched) {
        let s = match l {
            Launched::Dry { line } => format!(
                "dry launch: [{}] in {}\n  {line}\n",
                spec.title,
                spec.dir.display()
            ),
            Launched::Opened {
                new_window,
                split,
                warnings,
                ..
            } => {
                let mut s = String::new();
                for w in warnings {
                    let _ = writeln!(s, "yo: {w}");
                }
                let panes = if *split {
                    "the session in left pane, shell in right"
                } else {
                    "the session alone; the shell is the workgroup's terminal peer"
                };
                let _ = if *new_window {
                    writeln!(s, "==> opened iTerm2 window '{}' ({panes})", spec.title)
                } else {
                    writeln!(
                        s,
                        "==> opened iTerm2 tab '{}' in the '{}' window ({panes})",
                        spec.title, spec.group
                    )
                };
                s
            }
        };
        self.put(&s);
    }
}
