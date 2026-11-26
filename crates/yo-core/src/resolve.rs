//! `yo NAME`: what NAME means, in one ordered lookup. Pure over the
//! registry, the ledger and the live sessions; the caller acts on it. In
//! bash this order was spread over `main`, `cmd_reach` and
//! `cmd_home_reach`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::harness::LiveSession;
use crate::ledger::{Entry, State, state_of};
use crate::registry::Registry;

#[derive(Clone, Debug, PartialEq)]
pub enum Resolved {
    /// Create a session: NAME is a repo or profile (positionally), a flag
    /// was given, or nothing else matched.
    New,
    /// A registered agent: its home (`yo agent NAME`).
    Agent(String),
    /// A live session, by the harness's own name for it.
    Live(LiveSession),
    /// A ledger session, by yo name, id or 8+ character id prefix, that's
    /// running or can be resumed.
    Ledger(Box<Entry>, State),
    /// NAME's home: a repo keyed `NAME-home`, or whose clone is named that.
    Home { repo: String, target: HomeTarget },
}

/// The session `yo NAME` reaches in a home repo's clone.
#[derive(Clone, Debug, PartialEq)]
pub enum HomeTarget {
    /// A ledger session running there, or the latest ready one.
    Ledger(Box<Entry>, State),
    /// A live session there the ledger doesn't know.
    Live(LiveSession),
    /// None: start one in place.
    New,
}

/// More than one session matches.
#[derive(Clone, Debug, PartialEq)]
pub struct Ambiguous {
    pub query: String,
    /// The ledger's matches; empty when it's live sessions that share a name.
    pub entries: Vec<Entry>,
    pub live: usize,
}

/// `args` are the positional arguments; `has_flags` whether any flag was
/// given (which always means create).
pub fn resolve(
    reg: &Registry,
    entries: &[Entry],
    live: &[LiveSession],
    args: &[String],
    has_flags: bool,
) -> Result<Resolved, Ambiguous> {
    let (Some(name), 1..=2) = (args.first(), args.len()) else {
        return Ok(Resolved::New);
    };
    if has_flags || reg.repo(name).is_some() || reg.profile(name).is_some() {
        return Ok(Resolved::New);
    }
    if reg.agent(name).is_some() {
        return Ok(Resolved::Agent(name.clone()));
    }

    // A live session by its own name.
    let named: Vec<&LiveSession> = live
        .iter()
        .filter(|l| l.name.as_deref() == Some(name))
        .collect();
    match named.as_slice() {
        [one] => return Ok(Resolved::Live((*one).clone())),
        [] => {}
        many => {
            return Err(Ambiguous {
                query: name.clone(),
                entries: Vec::new(),
                live: many.len(),
            });
        }
    }

    // A ledger session by yo name or id, else by id prefix.
    let mut hits: Vec<&Entry> = entries
        .iter()
        .filter(|e| {
            e.record.session_id.as_deref() == Some(name) || e.record.name.as_deref() == Some(name)
        })
        .collect();
    if hits.is_empty() && name.len() >= 8 {
        hits = entries
            .iter()
            .filter(|e| {
                e.record
                    .session_id
                    .as_deref()
                    .is_some_and(|id| id.starts_with(name.as_str()))
            })
            .collect();
    }
    match hits.as_slice() {
        [] => {}
        [one] => {
            let st = state_of(&one.record, live);
            // Gone, or no id: nothing to reach, so NAME falls through.
            if matches!(st, State::Running { .. } | State::Ready | State::Ended) {
                return Ok(Resolved::Ledger(Box::new((*one).clone()), st));
            }
        }
        many => {
            return Err(Ambiguous {
                query: name.clone(),
                entries: many.iter().map(|e| (*e).clone()).collect(),
                live: 0,
            });
        }
    }

    if let Some(repo) = home_repo(reg, name) {
        let dir = reg.repo(&repo).map(|r| r.path.clone()).unwrap_or_default();
        return Ok(Resolved::Home {
            target: home_target(&dir, entries, live),
            repo,
        });
    }
    Ok(Resolved::New)
}

/// The repo that's NAME's home: keyed `NAME-home`, else the first whose
/// clone's directory is named that.
fn home_repo(reg: &Registry, name: &str) -> Option<String> {
    let want = format!("{name}-home");
    if reg.repo(&want).is_some() {
        return Some(want);
    }
    reg.repos()
        .iter()
        .find(|r| r.path.file_name().is_some_and(|f| *f == *want))
        .map(|r| r.key.clone())
}

fn real(p: &Path) -> PathBuf {
    fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// In a home's clone: a ledger session running there, else a live one the
/// ledger doesn't know, else the most recently used ready one. One quit
/// with `/exit` is left alone, so the next `yo NAME` starts fresh.
fn home_target(dir: &Path, entries: &[Entry], live: &[LiveSession]) -> HomeTarget {
    let d = real(dir);
    let mut here: Vec<(&Entry, State)> = entries
        .iter()
        .filter(|e| real(&e.record.worktree) == d)
        .map(|e| (e, state_of(&e.record, live)))
        .collect();
    here.sort_by_key(|(e, _)| {
        std::cmp::Reverse(e.record.updated_at.or(e.record.created_at).unwrap_or(0))
    });
    if let Some((e, st)) = here
        .iter()
        .find(|(_, s)| matches!(s, State::Running { .. }))
    {
        return HomeTarget::Ledger(Box::new((*e).clone()), *st);
    }
    if let Some(l) = live
        .iter()
        .filter(|l| real(&l.cwd) == d)
        .max_by_key(|l| l.updated_at)
    {
        return HomeTarget::Live(l.clone());
    }
    if let Some((e, st)) = here.iter().find(|(_, s)| *s == State::Ready) {
        return HomeTarget::Ledger(Box::new((*e).clone()), *st);
    }
    HomeTarget::New
}

#[cfg(test)]
mod tests;
