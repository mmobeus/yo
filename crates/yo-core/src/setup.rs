//! `yo setup`: what yo needs on a machine besides its binary. The
//! harnesses' files (Claude's skills, settings layer and statusline), from
//! copies built into yo, or linked into a checkout with `--link`; yo's
//! starting config, made once; optionally the harness's settings merged
//! into its own; and iTerm2's workgroup peer. Safe to run again: what's in
//! place is left alone, anything else in the way is backed up first, and
//! `--uninstall` takes out only what yo put there.
//!
//! What it installed is recorded in a manifest in yo's state directory, so
//! a later run knows its own copies from files someone edited, and
//! [`stale`] can tell when an upgraded yo has newer ones.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::harness::{self, Bundle, Bundled, BundledKind, GlobalHook};
use crate::ojson::OValue;
use crate::paths::Paths;
use crate::plan::Note;
use crate::runner::iterm;
use crate::sys::{CommandSpec, Sys};
use crate::{Error, Result, naming};

#[cfg(test)]
mod tests;

/// The workgroup peer's name in iTerm2.
const PEER_NAME: &str = "CLI";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetupOpts {
    /// Take out what setup put in.
    pub uninstall: bool,
    /// Merge a harness's settings into its own settings file (with
    /// `uninstall`: take the merged keys back out).
    pub merge_settings: bool,
    /// Add iTerm2's workgroup peer.
    pub iterm: bool,
    /// Replace links that point somewhere unexpected.
    pub force: bool,
    /// Link the files into this yo checkout instead of copying them.
    pub link: Option<PathBuf>,
    /// Add yo's ledger hook to the harnesses' own hooks files (Codex's).
    /// One that's there already is kept current either way.
    pub hooks: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SetupPlan {
    /// The checkout `--link` named, resolved.
    pub checkout: Option<PathBuf>,
    pub notes: Vec<Note>,
    pub steps: Vec<Step>,
    /// Hooks the harness won't run until the person approves them there.
    pub approve: Vec<Approve>,
}

/// A hook to approve in a harness, once setup has put it in place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Approve {
    /// The harness, and its command.
    pub harness: String,
    pub command: String,
    /// What the person does there.
    pub how: &'static str,
}

/// A hook setup can add, when asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub harness: String,
    pub what: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Nothing to do here.
    Keep {
        path: PathBuf,
        why: Kept,
    },
    /// Saves what's at `path` as `to` before it's replaced: moved aside, or
    /// copied (`copy`) when the file is edited in place.
    Backup {
        path: PathBuf,
        to: PathBuf,
        copy: bool,
    },
    Remove {
        path: PathBuf,
        why: Removed,
    },
    /// Writes one of yo's built-in files (or directories).
    Copy(Bundled),
    Link {
        path: PathBuf,
        to: PathBuf,
    },
    /// Writes a JSON file.
    Write {
        path: PathBuf,
        contents: String,
        what: Wrote,
    },
    /// Adds a terminal peer to iTerm2's Claude Code workgroup, saving the
    /// workgroups as they were to `backup`.
    AddPeer {
        name: String,
        old: String,
        new: String,
        backup: PathBuf,
        /// iTerm2 is running, so it sees the peer after a restart.
        running: bool,
    },
    /// The workgroup has a terminal peer already.
    PeerPresent,
    /// Records what's installed (`None`: forgets it).
    Manifest {
        path: PathBuf,
        contents: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kept {
    /// yo's copy, as this yo has it.
    Installed,
    /// Linked where it should be.
    Linked,
    /// A link somewhere yo didn't make (`--force` replaces it).
    Foreign { to: PathBuf },
    /// Uninstalling: changed since yo installed it.
    Edited,
    /// yo's config is yours, and stays.
    Config,
    /// The settings are merged already.
    Merged,
    /// yo's hook is in the harness's hooks file, as this yo has it.
    Hooked,
    /// An old link to a checkout's yo, which is the yo running now.
    RunningYo { to: PathBuf },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Removed {
    /// A link into a yo checkout (an older install, or `--link`).
    Link { to: PathBuf },
    /// A link somewhere else, replaced with `--force`.
    Forced { to: PathBuf },
    /// One of yo's copies: an older one being replaced, or uninstalling.
    Copy,
    /// A link to nothing, left by an older install.
    Dangling { to: PathBuf },
    /// An old link to a checkout's build of yo (or its bash script).
    OldYo { to: PathBuf },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Wrote {
    /// yo's starting config, naming the checkout when there is one.
    Config {
        checkout: Option<PathBuf>,
    },
    /// A harness's settings file, made from its layer.
    Settings,
    /// A harness's settings file, with these top-level keys merged in.
    Merged {
        keys: Vec<String>,
    },
    /// A harness's settings file, with these top-level keys taken out.
    Unmerged {
        keys: Vec<String>,
    },
    /// A harness's hooks file, with yo's ledger hook (or without it).
    Hook {
        harness: String,
    },
    Unhook {
        harness: String,
    },
}

/// What setup installed: each path, with a hash of what it wrote there,
/// `link` for a link, or `kept` for something it left alone.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Manifest {
    files: BTreeMap<String, String>,
}

const LINK: &str = "link";
const KEPT: &str = "kept";

/// What `yo setup` would do.
pub fn plan(sys: &dyn Sys, paths: &Paths, opts: &SetupOpts) -> Result<SetupPlan> {
    let bundles: Vec<Bundle> = harness::all()
        .iter()
        .map(|h| h.bundle(sys, paths))
        .collect();
    let items: Vec<&Bundled> = bundles.iter().flat_map(|b| &b.items).collect();
    let root = match &opts.link {
        Some(c) if !opts.uninstall => Some(checkout(c, &items)?),
        _ => None,
    };
    let stamp = naming::stamp(sys).replace('_', "-");
    let ctx = Plan {
        opts,
        had: read_manifest(paths).unwrap_or_default(),
        stamp: &stamp,
    };
    let mut p = SetupPlan {
        checkout: root.clone(),
        ..SetupPlan::default()
    };
    let mut now = Manifest::default();

    if !opts.uninstall {
        preflight(sys, &mut p.notes);
    }
    for item in &items {
        if opts.uninstall {
            ctx.uninstall_item(item, &mut p.steps);
        } else {
            let to = root.as_ref().map(|r| r.join(item.rel));
            let got = ctx.install_item(item, to, &mut p.steps);
            now.files.insert(key(&item.dest), got.to_string());
        }
    }
    ctx.config(paths, root.as_deref(), &mut p.steps);
    if opts.merge_settings {
        for g in bundles.iter().filter_map(|b| b.global.as_ref()) {
            ctx.merge(&g.path, g.layer, &mut p.steps)?;
        }
    }
    if opts.iterm && !opts.uninstall {
        match iterm::peer(sys, PEER_NAME) {
            iterm::Peer::Present => p.steps.push(Step::PeerPresent),
            iterm::Peer::NoGroup => p.notes.push(Note::Warn(
                "iTerm2's Claude Code workgroup isn't in its preferences, so no terminal peer \
                 was added; set the integration up in iTerm2 and run yo setup again (until \
                 then yo splits a pane for the shell)"
                    .into(),
            )),
            iterm::Peer::Missing { old, new } => p.steps.push(Step::AddPeer {
                name: PEER_NAME.into(),
                old,
                new,
                backup: paths
                    .config_dir
                    .join(format!("iterm-workgroups.bak-{stamp}.json")),
                running: iterm::running(sys),
            }),
        }
    }
    for h in harness::all() {
        if let Some(g) = h.global_hook(sys) {
            ctx.hook(h.name(), h.default_command(), &g, &mut p);
        }
    }
    old_yo(sys, opts, &mut p.steps);

    let manifest = paths.setup_manifest();
    if opts.uninstall {
        if manifest.is_file() {
            p.steps.push(Step::Manifest {
                path: manifest,
                contents: None,
            });
        }
    } else if now != ctx.had || !manifest.is_file() {
        let contents = serde_json::to_string_pretty(&now)
            .map_err(|e| Error::Failed(format!("the setup manifest: {e}")))?;
        p.steps.push(Step::Manifest {
            path: manifest,
            contents: Some(contents + "\n"),
        });
    }
    Ok(p)
}

/// The harnesses' hooks setup can add (with [`SetupOpts::hooks`]): those
/// not in place yet, for a harness that's installed.
pub fn hook_offers(sys: &dyn Sys) -> Vec<Offer> {
    harness::all()
        .iter()
        .filter_map(|h| {
            let g = h.global_hook(sys)?;
            (g.installed.is_none() && g.with.is_some() && runs(sys, h.default_command())).then(
                || Offer {
                    harness: h.name().to_string(),
                    what: g.offer,
                },
            )
        })
        .collect()
}

/// Whether harness `name`'s hook is in place and approved.
pub fn hook_approved(sys: &dyn Sys, name: &str) -> bool {
    harness::all()
        .iter()
        .filter(|h| h.name() == name)
        .filter_map(|h| h.global_hook(sys))
        .any(|g| g.installed.as_deref() == Some(g.command.as_str()) && g.approved)
}

/// Whether `program --version` runs.
fn runs(sys: &dyn Sys, program: &str) -> bool {
    sys.cmd()
        .run(&CommandSpec::new(program).arg("--version"))
        .is_ok_and(|o| o.status == Some(0))
}

/// Carries out `plan`, calling `each` before each step, and stopping at the
/// first that fails.
pub fn execute(sys: &dyn Sys, plan: &SetupPlan, each: &mut dyn FnMut(&Step)) -> Result<()> {
    for step in &plan.steps {
        each(step);
        match step {
            Step::Keep { .. } | Step::PeerPresent => {}
            Step::Backup { path, to, copy } => {
                let done = if *copy {
                    fs::copy(path, to).map(|_| ())
                } else {
                    fs::rename(path, to)
                };
                done.map_err(|e| Error::io(path, e))?;
            }
            Step::Remove { path, .. } => remove(path)?,
            Step::Copy(item) => write_bundled(item)?,
            Step::Link { path, to } => {
                if let Some(dir) = path.parent() {
                    fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
                }
                std::os::unix::fs::symlink(to, path).map_err(|e| Error::io(path, e))?;
            }
            Step::Write { path, contents, .. } => crate::write_atomic(path, contents.as_bytes())?,
            Step::AddPeer {
                old, new, backup, ..
            } => {
                crate::write_atomic(backup, old.as_bytes())?;
                iterm::write_workgroups(sys, new)?;
            }
            Step::Manifest { path, contents } => match contents {
                Some(c) => crate::write_atomic(path, c.as_bytes())?,
                None => match fs::remove_file(path) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                        return Err(Error::io(path, e));
                    }
                    _ => {}
                },
            },
        }
    }
    Ok(())
}

/// Whether a newer yo has different copies of the files setup installed
/// (or new ones): run setup again. `false` when setup hasn't run, or linked
/// them.
pub fn stale(sys: &dyn Sys, paths: &Paths) -> bool {
    let Some(m) = read_manifest(paths) else {
        return false;
    };
    harness::all().iter().any(|h| {
        h.bundle(sys, paths).items.iter().any(|item| {
            match m.files.get(&key(&item.dest)).map(String::as_str) {
                Some(LINK | KEPT) => false,
                Some(h) => h != hash(&item.kind),
                None => true,
            }
        })
    })
}

/// What planning needs throughout.
struct Plan<'a> {
    opts: &'a SetupOpts,
    /// The manifest from the last run.
    had: Manifest,
    stamp: &'a str,
}

impl Plan<'_> {
    fn backup(&self, path: &Path) -> PathBuf {
        let mut s = path.as_os_str().to_owned();
        s.push(format!(".bak-{}", self.stamp));
        PathBuf::from(s)
    }

    /// Whether what's at `dest` is a copy yo wrote: this yo's, or the one
    /// the last run recorded.
    fn ours(&self, item: &Bundled) -> bool {
        disk_hash(&item.dest, &item.kind).is_some_and(|h| {
            h == hash(&item.kind) || self.had.files.get(&key(&item.dest)) == Some(&h)
        })
    }

    /// Puts `item` in place, as a link to `to` or a copy; returns what the
    /// manifest records for it.
    fn install_item(&self, item: &Bundled, to: Option<PathBuf>, steps: &mut Vec<Step>) -> String {
        let dest = item.dest.clone();
        let want = hash(&item.kind);
        let put = |steps: &mut Vec<Step>| match &to {
            Some(t) => {
                steps.push(Step::Link {
                    path: dest.clone(),
                    to: t.clone(),
                });
                LINK.to_string()
            }
            None => {
                steps.push(Step::Copy(item.clone()));
                want.clone()
            }
        };
        let Ok(meta) = fs::symlink_metadata(&dest) else {
            return put(steps);
        };
        if meta.file_type().is_symlink() {
            let cur = fs::read_link(&dest).unwrap_or_default();
            if to.as_ref() == Some(&cur) {
                steps.push(Step::Keep {
                    path: dest,
                    why: Kept::Linked,
                });
                return LINK.into();
            }
            let why = if cur.ends_with(item.rel) {
                Removed::Link { to: cur }
            } else if self.opts.force {
                Removed::Forced { to: cur }
            } else {
                steps.push(Step::Keep {
                    path: dest,
                    why: Kept::Foreign { to: cur },
                });
                return KEPT.into();
            };
            steps.push(Step::Remove {
                path: dest.clone(),
                why,
            });
            return put(steps);
        }
        if to.is_none() && disk_hash(&dest, &item.kind).as_ref() == Some(&want) {
            steps.push(Step::Keep {
                path: dest,
                why: Kept::Installed,
            });
            return want;
        }
        if self.ours(item) {
            steps.push(Step::Remove {
                path: dest.clone(),
                why: Removed::Copy,
            });
        } else {
            steps.push(Step::Backup {
                path: dest.clone(),
                to: self.backup(&dest),
                copy: false,
            });
        }
        put(steps)
    }

    fn uninstall_item(&self, item: &Bundled, steps: &mut Vec<Step>) {
        let dest = item.dest.clone();
        let Ok(meta) = fs::symlink_metadata(&dest) else {
            return;
        };
        let step = if meta.file_type().is_symlink() {
            let cur = fs::read_link(&dest).unwrap_or_default();
            if cur.ends_with(item.rel) {
                Step::Remove {
                    path: dest,
                    why: Removed::Link { to: cur },
                }
            } else {
                Step::Keep {
                    path: dest,
                    why: Kept::Foreign { to: cur },
                }
            }
        } else if self.ours(item) {
            Step::Remove {
                path: dest,
                why: Removed::Copy,
            }
        } else {
            Step::Keep {
                path: dest,
                why: Kept::Edited,
            }
        };
        steps.push(step);
    }

    /// yo's config: made once, never touched again. A link to nothing is
    /// what an older install left (it linked the config into the repo).
    fn config(&self, paths: &Paths, root: Option<&Path>, steps: &mut Vec<Step>) {
        let path = paths.config.clone();
        let link = fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink());
        if path.exists() {
            steps.push(Step::Keep {
                path,
                why: Kept::Config,
            });
            return;
        }
        if self.opts.uninstall {
            return;
        }
        if link {
            steps.push(Step::Remove {
                path: path.clone(),
                why: Removed::Dangling {
                    to: fs::read_link(&path).unwrap_or_default(),
                },
            });
        }
        let mut repos = OValue::object();
        if let Some(r) = root {
            repos.insert(
                "yo",
                OValue::Object(vec![(
                    "path".into(),
                    OValue::String(r.to_string_lossy().into_owned()),
                )]),
            );
        }
        steps.push(Step::Write {
            path,
            contents: OValue::Object(vec![("repos".into(), repos)]).to_pretty(),
            what: Wrote::Config {
                checkout: root.map(Path::to_path_buf),
            },
        });
    }

    /// Merges `layer` into the settings file at `path` (its keys winning,
    /// objects merged key by key, as jq's `*`), or takes its top-level keys
    /// back out. The file is never replaced, so the rest of it stays.
    fn merge(&self, path: &Path, layer: &str, steps: &mut Vec<Step>) -> Result<()> {
        let layer = OValue::parse(layer)?;
        let keys: Vec<String> = match &layer {
            OValue::Object(m) => m.iter().map(|(k, _)| k.clone()).collect(),
            _ => Vec::new(),
        };
        if !path.exists() {
            if !self.opts.uninstall {
                steps.push(Step::Write {
                    path: path.to_path_buf(),
                    contents: layer.to_pretty(),
                    what: Wrote::Settings,
                });
            }
            return Ok(());
        }
        let text = fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
        let cur = OValue::parse(&text)
            .ok()
            .filter(|v| matches!(v, OValue::Object(_)));
        let Some(cur) = cur else {
            return Err(Error::Refused(format!(
                "{} is not valid JSON; fix or move it first",
                path.display()
            )));
        };
        let mut next = cur.clone();
        let what = if self.opts.uninstall {
            let gone: Vec<String> = keys
                .into_iter()
                .filter(|k| next.remove(k).is_some())
                .collect();
            if gone.is_empty() {
                return Ok(());
            }
            Wrote::Unmerged { keys: gone }
        } else {
            deep_merge(&mut next, layer);
            if next == cur {
                steps.push(Step::Keep {
                    path: path.to_path_buf(),
                    why: Kept::Merged,
                });
                return Ok(());
            }
            Wrote::Merged { keys }
        };
        steps.push(Step::Backup {
            path: path.to_path_buf(),
            to: self.backup(path),
            copy: true,
        });
        steps.push(Step::Write {
            path: path.to_path_buf(),
            contents: next.to_pretty(),
            what,
        });
        Ok(())
    }
}

impl Plan<'_> {
    /// Harness `name`'s hooks file `g`: yo's hook added (when asked) or
    /// kept current (when it's there), or taken out on uninstall. Written
    /// from what the harness read, the file as it was backed up first.
    fn hook(&self, name: &str, command: &str, g: &GlobalHook, p: &mut SetupPlan) {
        let put = |contents: &str, what: Wrote, steps: &mut Vec<Step>| {
            if g.path.exists() {
                steps.push(Step::Backup {
                    path: g.path.clone(),
                    to: self.backup(&g.path),
                    copy: true,
                });
            }
            steps.push(Step::Write {
                path: g.path.clone(),
                contents: contents.to_string(),
                what,
            });
        };
        if self.opts.uninstall {
            if let Some(w) = &g.without {
                put(
                    w,
                    Wrote::Unhook {
                        harness: name.into(),
                    },
                    &mut p.steps,
                );
            }
            return;
        }
        if g.installed.is_none() && !self.opts.hooks {
            return;
        }
        let current = g.installed.as_deref() == Some(g.command.as_str());
        if current {
            p.steps.push(Step::Keep {
                path: g.path.clone(),
                why: Kept::Hooked,
            });
        } else if let Some(w) = &g.with {
            put(
                w,
                Wrote::Hook {
                    harness: name.into(),
                },
                &mut p.steps,
            );
        } else {
            p.notes.push(Note::Warn(format!(
                "{} isn't shaped like a hooks file, so yo's hook wasn't added; fix or move it first",
                g.path.display()
            )));
            return;
        }
        // A changed command is a new hook to the harness.
        if !(current && g.approved) {
            p.approve.push(Approve {
                harness: name.into(),
                command: command.into(),
                how: g.approve,
            });
        }
    }
}

/// `~/.local/bin/yo`, linked to a checkout's build (or the bash script
/// before it) by `install.sh`. It would shadow or duplicate the yo
/// installed now, so it goes, unless it's the yo that's running.
fn old_yo(sys: &dyn Sys, opts: &SetupOpts, steps: &mut Vec<Step>) {
    let path = sys.home().join(".local/bin/yo");
    let Ok(to) = fs::read_link(&path) else {
        return;
    };
    let old = ["target/release/yo", "target/debug/yo", "scripts/yo"]
        .iter()
        .any(|t| to.ends_with(t));
    if !old {
        return;
    }
    let running =
        fs::canonicalize(&path).is_ok_and(|p| fs::canonicalize(sys.exe()).is_ok_and(|e| e == p));
    if running && !opts.uninstall {
        steps.push(Step::Keep {
            path,
            why: Kept::RunningYo { to },
        });
    } else {
        steps.push(Step::Remove {
            path,
            why: Removed::OldYo { to },
        });
    }
}

/// The checkout `--link` names, resolved, once it's sure to hold every
/// file.
fn checkout(c: &Path, items: &[&Bundled]) -> Result<PathBuf> {
    let root =
        fs::canonicalize(c).map_err(|e| Error::Refused(format!("--link {}: {e}", c.display())))?;
    if let Some(miss) = items.iter().find(|i| !root.join(i.rel).exists()) {
        return Err(Error::Refused(format!(
            "{} isn't a yo checkout (no {})",
            root.display(),
            miss.rel
        )));
    }
    Ok(root)
}

/// What's worth knowing before setting up: tools yo needs that aren't
/// there.
fn preflight(sys: &dyn Sys, notes: &mut Vec<Note>) {
    let has = |p: &str| {
        sys.cmd()
            .run(&CommandSpec::new(p).arg("--version"))
            // 127: what a shell says for a command it can't find.
            .is_ok_and(|o| o.status != Some(127))
    };
    for (tool, what) in [
        ("git", "yo needs it for worktrees"),
        ("jq", "the statusline needs it"),
        ("gh", "the parts of yo that use it will degrade"),
    ] {
        if !has(tool) {
            notes.push(Note::Warn(format!("{tool} not found: {what}")));
        }
    }
}

/// Objects merged key by key, `b` winning; anything else replaced (jq's
/// `*`).
fn deep_merge(a: &mut OValue, b: OValue) {
    let OValue::Object(b) = b else { return };
    for (k, v) in b {
        match (a.get_mut(&k), v) {
            (Some(x @ OValue::Object(_)), v @ OValue::Object(_)) => deep_merge(x, v),
            (_, v) => {
                a.insert(&k, v);
            }
        }
    }
}

fn write_bundled(item: &Bundled) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    match &item.kind {
        BundledKind::File { text, exec } => {
            crate::write_atomic(&item.dest, text.as_bytes())?;
            let mode = if *exec { 0o755 } else { 0o644 };
            fs::set_permissions(&item.dest, fs::Permissions::from_mode(mode))
                .map_err(|e| Error::io(&item.dest, e))
        }
        BundledKind::Dir(files) => {
            for (name, text) in files {
                crate::write_atomic(&item.dest.join(name), text.as_bytes())?;
            }
            Ok(())
        }
    }
}

/// A link or file goes; a directory goes with what's in it (only ever one
/// of yo's own, checked while planning).
fn remove(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path).map_err(|e| Error::io(path, e))?;
    let done = if meta.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    done.map_err(|e| Error::io(path, e))
}

fn read_manifest(paths: &Paths) -> Option<Manifest> {
    let text = fs::read_to_string(paths.setup_manifest()).ok()?;
    serde_json::from_str(&text).ok()
}

fn key(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// A hash of what an item holds (64-bit FNV-1a: telling yo's copies from
/// edited ones, not security), as hex.
fn hash(kind: &BundledKind) -> String {
    match kind {
        BundledKind::File { text, .. } => fnv(&[text.as_bytes()]),
        BundledKind::Dir(files) => {
            let parts: Vec<&[u8]> = files
                .iter()
                .flat_map(|(n, t)| [n.as_bytes(), t.as_bytes()])
                .collect();
            fnv(&parts)
        }
    }
}

/// The same hash of what's on disk at `dest`, if it's the same shape: a
/// file, or a directory holding exactly those files.
fn disk_hash(dest: &Path, kind: &BundledKind) -> Option<String> {
    match kind {
        BundledKind::File { .. } => {
            let meta = fs::symlink_metadata(dest).ok()?;
            if !meta.is_file() {
                return None;
            }
            Some(fnv(&[&fs::read(dest).ok()?]))
        }
        BundledKind::Dir(files) => {
            let meta = fs::symlink_metadata(dest).ok()?;
            if !meta.is_dir() || fs::read_dir(dest).ok()?.count() != files.len() {
                return None;
            }
            let mut held = Vec::new();
            for (n, _) in files {
                held.push(n.as_bytes().to_vec());
                held.push(fs::read(dest.join(n)).ok()?);
            }
            let parts: Vec<&[u8]> = held.iter().map(Vec::as_slice).collect();
            Some(fnv(&parts))
        }
    }
}

fn fnv(parts: &[&[u8]]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        for b in p.iter().chain(&[0u8]) {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{h:016x}")
}
