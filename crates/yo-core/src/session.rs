//! Finding and recording sessions for the ledger commands (`resume`,
//! `adopt`, `forget`): where a directory sits in yo's world, and a session
//! by id, prefix, directory or name, falling back to what the harness
//! itself recorded.

use std::path::{Path, PathBuf};

use crate::ledger::{Adopt, Entry};
use crate::registry::Mode;
use crate::{Ctx, Result};

/// Where a directory sits: a yo worktree (its root), a registered clone, or
/// neither. bash's `_yo_classify_dir`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Classified {
    /// A yo worktree's root, else the directory as given.
    pub dir: PathBuf,
    pub repo: Option<String>,
    pub name: String,
    /// `Worktree` only for a yo worktree.
    pub mode: Mode,
    pub title: String,
}

pub fn classify(ctx: &Ctx, dir: &Path) -> Classified {
    let reg = &ctx.registry;
    let d = format!("{}/", dir.display());
    let mut c = Classified {
        dir: dir.to_path_buf(),
        repo: None,
        name: String::new(),
        mode: Mode::Here,
        title: String::new(),
    };
    for r in reg.repos() {
        let clone = r.path.display().to_string();
        let root = format!("{clone}-worktrees/");
        if let Some(rest) = d.strip_prefix(&root) {
            let name = rest.split('/').next().unwrap_or("").to_string();
            c.repo = Some(r.key.clone());
            c.dir = PathBuf::from(format!("{root}{name}"));
            c.name = name;
            c.mode = Mode::Worktree;
            break;
        }
        if d.starts_with(&format!("{clone}/")) {
            c.repo = Some(r.key.clone());
            c.name = basename(&r.path);
        }
    }
    if c.name.is_empty() {
        c.name = basename(dir);
    }
    let repo = c.repo.as_deref().and_then(|k| reg.repo(k));
    let prefix = match repo {
        Some(r) => reg.title_prefix(r),
        None => format!("[{}] ", basename(&c.dir)),
    };
    c.title = format!("{prefix}{}", c.name);
    c
}

fn basename(p: &Path) -> String {
    p.file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A session to act on.
#[derive(Clone, Debug, PartialEq)]
pub enum Found {
    Entry(Box<Entry>),
    /// Not in the ledger, but a harness knows where session `id` ran.
    Unrecorded {
        id: String,
        dir: PathBuf,
        harness: String,
    },
}

/// The session `q` names: in the ledger (see [`crate::ledger::Ledger::get`]),
/// else an id a harness has a record of. `Err` lists the ledger's matches
/// when there's more than one.
pub fn find(ctx: &Ctx, q: &str) -> std::result::Result<Option<Found>, Vec<Entry>> {
    if let Some(e) = ctx.ledger().get(q)? {
        return Ok(Some(Found::Entry(Box::new(e))));
    }
    Ok(ctx.harnesses.iter().find_map(|h| {
        h.session_cwd(&*ctx.sys, q).map(|dir| Found::Unrecorded {
            id: q.to_string(),
            dir,
            harness: h.name().to_string(),
        })
    }))
}

/// Records a session from inside it, or one the ledger didn't have, so it
/// has a title and comes back with the rest on the next `resume-all`.
/// `title` names a new record; a `given` one (`--title`) also replaces
/// what's recorded.
pub fn record(
    ctx: &Ctx,
    id: &str,
    dir: &Path,
    harness: &str,
    title: Option<&str>,
    given: bool,
) -> Result<Entry> {
    let c = classify(ctx, dir);
    let repo = c.repo.as_deref().and_then(|k| ctx.registry.repo(k));
    let a = Adopt {
        dir: c.dir.clone(),
        id: id.to_string(),
        title: title.map_or(c.title.clone(), str::to_string),
        title_given: given && title.is_some(),
        repo: c.repo.clone(),
        name: c.name.clone(),
        profile: ctx.registry.profile_of(repo, None).map(|p| p.name.clone()),
        mode: c.mode,
        harness: Some(harness.to_string()),
    };
    let ledger = ctx.ledger();
    let key = ledger.adopt(&a, &ctx.live())?;
    Ok(ledger
        .all()
        .into_iter()
        .find(|e| e.key == key)
        .unwrap_or(Entry {
            key,
            record: Default::default(),
        }))
}

/// The sessions started to report back to `session` (`--from`): those
/// whose `from` is any of its names, the one given or another for the same
/// session (its harness's name, its yo name, its id), so it matches
/// however it was named. Not the session itself.
pub fn started_from(ctx: &Ctx, session: &str) -> Vec<Entry> {
    let all = ctx.ledger().all();
    let live = ctx.live();
    // Its ids, by any name it goes by; then every name those ids have.
    let mut ids: Vec<String> = vec![session.to_string()];
    for l in &live {
        if l.name.as_deref() == Some(session) {
            ids.push(l.id.clone());
        }
    }
    for e in &all {
        if e.record.name.as_deref() == Some(session) {
            ids.extend(e.record.id().map(str::to_string));
        }
    }
    let mut names = ids.clone();
    for l in live.iter().filter(|l| ids.contains(&l.id)) {
        names.extend(l.name.clone());
    }
    for e in &all {
        if e.record.id().is_some_and(|id| ids.iter().any(|i| i == id)) {
            names.extend(e.record.name.clone());
        }
    }
    all.into_iter()
        .filter(|e| {
            !e.record
                .id()
                .is_some_and(|id| names.iter().any(|n| n == id))
        })
        .filter(|e| e.record.from.as_ref().is_some_and(|f| names.contains(f)))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use super::*;
    use crate::fake::{FakeSys, TempDir};

    fn ctx(t: &TempDir) -> Ctx {
        let h = t.path().display();
        let cfg = t.path().join("c.json");
        fs::write(
            &cfg,
            format!(r#"{{"repos": {{"api": {{"path": "{h}/src/api"}}, "web": {{"path": "{h}/src/web"}}}}}}"#),
        )
        .unwrap();
        let sys = Arc::new(FakeSys::new(t.path()));
        sys.set_env("YO_CONFIG", &cfg.to_string_lossy());
        Ctx::load(sys, t.path()).unwrap()
    }

    #[test]
    fn directories() {
        let t = TempDir::new().unwrap();
        let c = ctx(&t);
        let h = t.path();
        let got = classify(&c, &h.join("src/web-worktrees/feat/sub/dir"));
        assert_eq!(got.dir, h.join("src/web-worktrees/feat"));
        assert_eq!(
            (got.repo.as_deref(), got.name.as_str(), got.mode),
            (Some("web"), "feat", Mode::Worktree)
        );
        assert_eq!(got.title, "[web] feat");
        // In a clone: the clone's name, in place. The default repo has no prefix.
        let got = classify(&c, &h.join("src/api/lib"));
        assert_eq!(
            (got.repo.as_deref(), got.name.as_str(), got.mode),
            (Some("api"), "api", Mode::Here)
        );
        assert_eq!(got.dir, h.join("src/api/lib"));
        assert_eq!(got.title, "api");
        let got = classify(&c, &h.join("elsewhere/proj"));
        assert_eq!((got.repo, got.name.as_str()), (None, "proj"));
        assert_eq!(got.title, "[proj] proj");
    }

    #[test]
    fn found_in_the_ledger_or_by_the_harness() {
        let t = TempDir::new().unwrap();
        let c = ctx(&t);
        let id = "abcdef12-0000-4000-8000-000000000000";
        assert_eq!(find(&c, id), Ok(None));
        let proj = t.path().join(".claude/projects/-x");
        fs::create_dir_all(&proj).unwrap();
        fs::write(
            proj.join(format!("{id}.jsonl")),
            format!(
                "not json\n{{\"type\": \"x\"}}\n{{\"cwd\": \"{}/src/web-worktrees/f\"}}\n",
                t.path().display()
            ),
        )
        .unwrap();
        let Ok(Some(Found::Unrecorded { dir, harness, .. })) = find(&c, id) else {
            panic!()
        };
        assert_eq!(harness, "claude");
        let e = record(&c, id, &dir, &harness, None, false).unwrap();
        assert_eq!(
            e.key.0,
            format!(
                "{}",
                crate::ledger::Key::for_worktree(t.path(), &t.path().join("src/web-worktrees/f")).0
            )
        );
        assert_eq!(e.record.title.as_deref(), Some("[web] f"));
        assert!(matches!(find(&c, "abcdef12"), Ok(Some(Found::Entry(_)))));
        assert!(matches!(find(&c, "f"), Ok(Some(Found::Entry(_)))));
    }
}
