//! What the read-only commands print. Same text as `scripts/yo` prints
//! today, from the core's data.

use std::fmt::Write as _;

use yo_core::Ctx;
use yo_core::harness::LiveSession;
use yo_core::ledger::{Entry, State, state_of};
use yo_core::registry::Mode;

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
pub fn sessions(ctx: &Ctx, entries: &[Entry], live: &[LiveSession]) -> String {
    if entries.is_empty() {
        return format!(
            "no sessions in the ledger ({})\n",
            ctx.ledger().dir().display()
        );
    }
    let mut s = format!(
        "{:<8} {:<7} {:<12} {:<10} {:<36} NAME\n",
        "STATUS", "PID", "REPO", "AGENT", "SESSION"
    );
    for e in entries {
        let r = &e.record;
        let state = state_of(r, live);
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
            "{:<8} {:<7} {:<12} {:<10} {:<36} {name}",
            state.as_str(),
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
    s
}
