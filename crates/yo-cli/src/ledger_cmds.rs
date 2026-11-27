//! The ledger commands: `resume`, `resume-all`, `adopt`, `forget`, `title`.

use std::io::Write;
use std::path::{Path, PathBuf};

use yo_core::Ctx;
use yo_core::git;
use yo_core::ledger::{Entry, State, state_of};
use yo_core::plan::execute;
use yo_core::reach::{ResumeOpts, await_tabless, ensure_guest, plan_resume, tabless};
use yo_core::runner::iterm::Iterm;
use yo_core::session::{self, Found, classify};

use crate::cli::{Adopt, Resume, ResumeAll};
use crate::render;
use crate::run::{runner, say, shown, window};

type Res = Result<i32, Box<dyn std::error::Error>>;

/// A query that's a directory, as the ledger holds it: absolute and real.
pub(crate) fn as_dir(cwd: &Path, q: &str) -> String {
    let p = cwd.join(q);
    if p.is_dir()
        && let Ok(real) = std::fs::canonicalize(&p)
    {
        return real.display().to_string();
    }
    q.to_string()
}

/// More than one session matches.
pub(crate) fn ambiguous(ctx: &Ctx, q: &str, hits: &[Entry]) -> Box<dyn std::error::Error> {
    let mut s = format!("ambiguous: {} sessions match '{q}'\n", hits.len());
    let live = ctx.live();
    let table = render::sessions(ctx, hits, &live, shown(ctx, &live).as_ref());
    for l in table.lines().take(hits.len() + 1) {
        s.push_str(l);
        s.push('\n');
    }
    s.push_str("yo: be more specific (use the session id)");
    s.into()
}

fn sync(ctx: &Ctx, out: &mut dyn Write) {
    if let Ok(adopted) = ctx.ledger().sync(&ctx.sync_clones(), &ctx.live()) {
        for a in adopted {
            let _ = writeln!(out, "adopted {} ({}) from a live session", a.name, a.id);
        }
    }
}

/// `yo resume [QUERY]`: reopens a session by name, id, 8+ prefix or
/// directory; with no query, lists them.
pub fn resume(ctx: &mut Ctx, r: &Resume, cwd: &Path, out: &mut dyn Write) -> Res {
    let Some(q) = &r.query else {
        sync(ctx, out);
        let live = ctx.live();
        let table = render::sessions(ctx, &ctx.ledger().all(), &live, shown(ctx, &live).as_ref());
        out.write_all(table.as_bytes())?;
        return Ok(0);
    };
    let q = as_dir(cwd, q);
    sync(ctx, out);
    let e = match session::find(ctx, &q).map_err(|hits| ambiguous(ctx, &q, &hits))? {
        None => return Err(format!("no session matches '{q}' (yo sessions lists them)").into()),
        Some(Found::Entry(e)) => *e,
        // Known only to the harness: record it, so it has a title and comes
        // back with the rest on the next resume-all.
        Some(Found::Unrecorded { id, dir, harness }) => {
            session::record(ctx, &id, &dir, &harness, None, false)?
        }
    };
    let name = e.record.name.clone().unwrap_or_else(|| e.key.0.clone());
    let live = ctx.live();
    match state_of(&e.record, &live) {
        State::Running { pid } => {
            if tabless(ctx, &e.record, &live, shown(ctx, &live).as_ref()).is_some() {
                return Err(format!(
                    "{name} is still running (pid {pid}), but its tab was closed; end it with kill -HUP {pid}, then yo resume {name}"
                )
                .into());
            }
            return Err(format!("{name} is already running (pid {pid})").into());
        }
        State::Gone => {
            return Err(format!(
                "{name}'s directory is gone: {}",
                e.record.worktree.display()
            )
            .into());
        }
        State::NoId => return Err(format!("{name} has no recorded session id to resume").into()),
        State::Ready | State::Ended => {}
    }
    say(out, &ensure_guest(ctx, &e)?);
    let plan = plan_resume(
        ctx,
        &e,
        &ResumeOpts {
            message: None,
            model: r.model.clone(),
            window: window(&r.window),
        },
    )?;
    let mut report = render::Progress::new(out);
    execute(ctx, &plan, &*runner(ctx), &mut report)?;
    report.finish()?;
    Ok(0)
}

/// `yo resume-all`: reopens every ready session (not those quit with
/// `/exit`).
pub fn resume_all(ctx: &mut Ctx, r: &ResumeAll, out: &mut dyn Write) -> Res {
    sync(ctx, out);
    let mut live = ctx.live();
    let shown = shown(ctx, &live);
    let entries = ctx.ledger().all();
    // Sessions whose tab just closed may still be exiting: given a moment,
    // they're resumed rather than skipped.
    let waiting = !r.dry_run
        && entries
            .iter()
            .any(|e| tabless(ctx, &e.record, &live, shown.as_ref()).is_some());
    if waiting {
        writeln!(
            out,
            "==> waiting for sessions whose tab just closed to exit"
        )?;
        if let Some(now) = await_tabless(ctx, &entries, shown.as_ref(), &live) {
            live = now;
        }
    }
    let mut n = 0;
    let mut skipped = String::new();
    let mut stuck = String::new();
    for e in entries {
        let st = state_of(&e.record, &live);
        let name = e.record.name.clone().unwrap_or_else(|| e.key.0.clone());
        if let Some(pid) = tabless(ctx, &e.record, &live, shown.as_ref()) {
            stuck.push_str(&format!("  {name}  (pid {pid})\n"));
            continue;
        }
        if st != State::Ready {
            skipped.push_str(&format!("  {}  {name}\n", st.as_str()));
            continue;
        }
        n += 1;
        if r.dry_run {
            writeln!(
                out,
                "would resume: {name} ({}) in {}",
                e.record.session_id.as_deref().unwrap_or("-"),
                e.record.worktree.display()
            )?;
            continue;
        }
        say(out, &ensure_guest(ctx, &e)?);
        let plan = plan_resume(
            ctx,
            &e,
            &ResumeOpts {
                window: window(&r.window),
                ..ResumeOpts::default()
            },
        )?;
        let mut report = render::Progress::new(out);
        execute(ctx, &plan, &*runner(ctx), &mut report)?;
    }
    if !stuck.is_empty() {
        write!(
            out,
            "\nNOT resumed: still running, though its tab was closed:\n{stuck}\
             end each with kill -HUP <pid>, then yo resume <name>\n\n"
        )?;
    }
    if !skipped.is_empty() {
        write!(out, "skipped:\n{skipped}")?;
    }
    if r.dry_run {
        writeln!(out, "==> {n} session(s) would be resumed")?;
    } else {
        writeln!(out, "==> resumed {n} session(s)")?;
    }
    Ok(0)
}

/// `yo adopt`: records the session it's run from (its id from the harness's
/// environment, or `--session-id`), so `resume-all` brings it back.
pub fn adopt(ctx: &Ctx, a: &Adopt, cwd: &Path, out: &mut dyn Write) -> Res {
    let sys = &*ctx.sys;
    let own = ctx.own_session_of();
    let id = a
        .session_id
        .clone()
        .or_else(|| own.as_ref().map(|(_, id)| id.clone()))
        .ok_or("no session id: run this from inside a coding session, or pass --session-id=ID")?;
    // The harness it runs inside, else the one a session here would run on.
    let harness = match own {
        Some((h, _)) => h,
        None => {
            let repo = ctx.registry.detect_repo(sys, cwd);
            ctx.harness(&ctx.registry.harness(None, repo, None))?
        }
    };
    let dir: PathBuf = harness
        .session_cwd(sys, &id)
        .unwrap_or_else(|| std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf()));
    // The title the tab already has, when yo titled it; else derived.
    let tab_title = sys
        .env_nonempty("ITERM_SESSION_ID")
        .and_then(|s| Iterm::own_title(sys, &s));
    // Only --title replaces a recorded title; the tab's names a new record.
    let title = a.title.clone().or(tab_title);
    let e = session::record(
        ctx,
        &id,
        &dir,
        harness.name(),
        title.as_deref(),
        a.title.is_some(),
    )?;
    // Its tab, so yo can tell later whether it still has one.
    if let Some(tab) = sys
        .env_nonempty("ITERM_SESSION_ID")
        .and_then(|s| Iterm::own_tab(sys, &s))
    {
        let _ = ctx.ledger().set_tab(&e.key, &tab);
    }
    let shown = e
        .record
        .title
        .clone()
        .unwrap_or_else(|| classify(ctx, &dir).title);
    writeln!(out, "==> recorded: {shown}")?;
    writeln!(out, "        session: {id}")?;
    writeln!(out, "        dir:     {}", e.record.worktree.display())?;
    writeln!(out, "    yo resume-all will bring it back after a restart.")?;
    Ok(0)
}

/// `yo forget QUERY`: drops a session from the ledger, nothing else.
pub fn forget(ctx: &Ctx, q: &str, cwd: &Path, out: &mut dyn Write) -> Res {
    let q = as_dir(cwd, q);
    let e = match ctx.ledger().get(&q) {
        Ok(Some(e)) => e,
        _ => {
            return Err(format!("no single session matches '{q}' (yo sessions lists them)").into());
        }
    };
    let name = e.record.name.clone().unwrap_or_else(|| e.key.0.clone());
    if matches!(state_of(&e.record, &ctx.live()), State::Running { .. }) {
        writeln!(
            out,
            "yo: note: {name} is still running; forgetting it only stops resume-all bringing it back"
        )?;
    }
    ctx.ledger().drop(&e.key)?;
    writeln!(
        out,
        "forgot {name} ({}); its transcript and directory are untouched",
        e.record.session_id.as_deref().unwrap_or("-")
    )?;
    Ok(0)
}

/// `yo title [TITLE]`: retitles this session's tab and its record; with no
/// title, puts every yo tab's title back.
pub fn title(ctx: &Ctx, title: Option<&str>, cwd: &Path, out: &mut dyn Write) -> Res {
    let sys = &*ctx.sys;
    let Some(title) = title.filter(|t| !t.is_empty()) else {
        Iterm::resync_titles(sys);
        writeln!(out, "==> session names reset to their yo titles")?;
        return Ok(0);
    };
    let iterm_id = sys
        .env_nonempty("ITERM_SESSION_ID")
        .ok_or("not inside an iTerm2 session — no tab to retitle")?;
    if !Iterm::retitle_own(sys, &iterm_id, title)? {
        return Err("couldn't find this session's iTerm tab".into());
    }
    let top = git::git(sys, cwd, &["rev-parse", "--show-toplevel"])
        .map(PathBuf::from)
        .unwrap_or_else(|| std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf()));
    let key = sys.env_nonempty("YO_SESSION_KEY");
    let id = ctx.own_session();
    if !ctx
        .ledger()
        .retitle(key.as_deref(), id.as_deref(), &top, title)
        .unwrap_or(false)
    {
        writeln!(
            out,
            "yo: (not in the ledger — the title won't survive a resume; yo adopt records it)"
        )?;
    }
    // The workgroup may rename the session again shortly; hold the name.
    Iterm::hold_title(sys, title);
    writeln!(out, "==> titled: {title}")?;
    Ok(0)
}
