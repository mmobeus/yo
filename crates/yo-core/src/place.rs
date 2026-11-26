//! A PR or issue URL whose repo isn't registered: where it goes. The profile
//! that claims its owner, else what `--add-to` (or the person, asked) says,
//! else a guest for this run. bash's `place_url_repo` and
//! `register_url_repo`.
//!
//! This runs before planning, because planning the worktree needs the
//! clone.

use std::fs;
use std::path::PathBuf;

use crate::config::Config;
use crate::plan::Note;
use crate::registry::{CloneProtocol, Mode, Repo, valid_key};
use crate::sys::CommandSpec;
use crate::{Ctx, Error, Result};

/// Where an unregistered repo goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaceTo {
    /// This run only: not pre-trusted, no shared memory.
    Guest,
    /// Into a profile, saved to the config so it's asked only once.
    Profile(String),
    /// A new profile, saved to the config, claiming the repo's owner.
    NewProfile(String),
}

impl PlaceTo {
    /// `--add-to`'s value: `guest`, or a profile.
    pub fn parse(ctx: &Ctx, v: &str) -> Result<PlaceTo> {
        if v == "guest" {
            return Ok(PlaceTo::Guest);
        }
        if ctx.registry.profile(v).is_some() {
            return Ok(PlaceTo::Profile(v.to_string()));
        }
        let names: Vec<&str> = ctx
            .registry
            .profiles()
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        Err(Error::Refused(format!(
            "--add-to must be a profile ({}) or guest (got: '{v}')",
            names.join(" ")
        )))
    }
}

/// What to offer when asking where `owner_repo` goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choices {
    pub profiles: Vec<String>,
    /// A new profile named after the owner, unless that name is taken.
    pub new_profile: Option<String>,
}

/// The profile that claims `owner_repo`'s owner: no need to ask.
pub fn claimed_by(ctx: &Ctx, owner_repo: &str) -> Option<String> {
    let owner = owner_repo.split('/').next()?;
    ctx.registry
        .profile_for_owner(owner)
        .map(|p| p.name.clone())
}

pub fn choices(ctx: &Ctx, owner_repo: &str) -> Choices {
    let reg = &ctx.registry;
    let owner = owner_repo.split('/').next().unwrap_or("");
    let taken = reg.profile(owner).is_some() || reg.repo(owner).is_some();
    Choices {
        profiles: reg.profiles().iter().map(|p| p.name.clone()).collect(),
        new_profile: (!taken && valid_key(owner)).then(|| owner.to_string()),
    }
}

/// Where `owner_repo`'s clone is, or would be.
pub fn clone_path(ctx: &Ctx, owner_repo: &str) -> PathBuf {
    ctx.registry.defaults.clone_root.join(owner_repo)
}

/// Clones `owner_repo` (unless it's already at its place under the clone
/// root) and registers it for this run where `to` says: the profile that
/// claims its owner wins over `to`. Returns its key, and what it did.
pub fn place(ctx: &mut Ctx, owner_repo: &str, to: &PlaceTo) -> Result<(String, Vec<Note>)> {
    let (owner, repo) = owner_repo
        .split_once('/')
        .filter(|(o, r)| valid_key(o) && valid_key(r) && !o.starts_with('.') && !r.starts_with('.'))
        .ok_or_else(|| {
            Error::Refused(format!(
                "can't use '{owner_repo}': unexpected characters in the repo name"
            ))
        })?;
    let mut notes = Vec::new();
    let claimed = claimed_by(ctx, owner_repo);
    let to = match &claimed {
        Some(p) => {
            notes.push(Note::Info(format!(
                "{owner_repo} belongs to profile '{p}' (it claims {owner})"
            )));
            PlaceTo::Profile(p.clone())
        }
        None => to.clone(),
    };

    let path = clone_path(ctx, owner_repo);
    if path.exists() {
        if !path.join(".git").exists() {
            return Err(Error::Refused(format!(
                "can't use {owner_repo}: {} exists and isn't a git clone",
                path.display()
            )));
        }
        notes.push(Note::Info(format!(
            "using the clone of {owner_repo} at {}",
            path.display()
        )));
    } else {
        let url = match ctx.registry.defaults.clone_protocol {
            CloneProtocol::Ssh => format!("git@github.com:{owner_repo}.git"),
            CloneProtocol::Https => format!("https://github.com/{owner_repo}.git"),
        };
        notes.push(Note::Info(format!(
            "cloning {owner_repo} to {}",
            path.display()
        )));
        if let Some(d) = path.parent() {
            fs::create_dir_all(d).map_err(|e| Error::io(d, e))?;
        }
        let spec = CommandSpec::new("git")
            .arg("clone")
            .arg(&url)
            .arg(path.to_string_lossy());
        let out = ctx.sys.cmd().run(&spec);
        if !out.as_ref().is_ok_and(|o| o.success()) {
            return Err(Error::Failed(format!(
                "couldn't clone {url}\n        (YO_CLONE_PROTOCOL picks ssh or https; YO_CLONE_ROOT picks where clones go)"
            )));
        }
    }

    let key = if ctx.registry.repo(repo).is_some() {
        format!("{owner}-{repo}")
    } else {
        repo.to_string()
    };
    let mut r = Repo::bare(key.clone(), path.clone());
    r.github = Some(owner_repo.to_string());
    let cfg = ctx.paths.config.clone();
    let has_cfg = cfg.is_file();
    match &to {
        PlaceTo::Guest => {
            r.mode = Some(Mode::Worktree);
            match ctx.registry.defaults.guest_profile.clone() {
                Some(p) => r.profile = Some(p),
                None => r.memory = Some("none".into()),
            }
            ctx.registry.guest = Some(key.clone());
        }
        PlaceTo::Profile(p) => {
            r.profile = Some(p.clone());
            // A profile that claims the owner needs nothing saved.
            if claimed.is_none() {
                if has_cfg {
                    match Config::add_repo(&cfg, &ctx.sys.home(), Some(p), &key, &path, Some(owner_repo)) {
                        Ok(()) => notes.push(Note::Info(format!(
                            "added {owner_repo} to profile '{p}' in {}",
                            cfg.display()
                        ))),
                        Err(e) => notes.push(Note::Warn(format!(
                            "couldn't save {owner_repo} to {} ({e}); it's in '{p}' for this run only",
                            cfg.display()
                        ))),
                    }
                } else {
                    notes.push(Note::Warn(format!(
                        "no config at {}, so {owner_repo} is in '{p}' for this run only",
                        cfg.display()
                    )));
                }
            }
        }
        PlaceTo::NewProfile(p) => {
            Config::add_profile(&cfg, p, owner)?;
            ctx.registry.add_profile(p, vec![owner.to_string()]);
            notes.push(Note::Info(format!(
                "new profile '{p}' (claims every {owner}/* repo), saved to {}",
                cfg.display()
            )));
            r.profile = Some(p.clone());
        }
    }
    ctx.registry.add_repo(r);
    Ok((key, notes))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::fake::{FakeSys, TempDir, failed};

    /// A config with profile `work` (claims `acme`) and profile `p`, and a
    /// clone root under the temporary HOME.
    fn ctx(t: &TempDir, extra_env: &[(&str, &str)]) -> (Ctx, Arc<FakeSys>) {
        let cfg = t.path().join("config.json");
        fs::write(
            &cfg,
            r#"{"profiles": {"work": {"owners": ["acme"]}, "p": {}, "u": {}},
                "repos": {"x": {"path": "/x"}}}"#,
        )
        .unwrap();
        let sys = Arc::new(FakeSys::new(t.path()));
        sys.set_env("YO_CONFIG", &cfg.to_string_lossy());
        sys.set_env("YO_CLONE_ROOT", &t.path().join("src").to_string_lossy());
        for (k, v) in extra_env {
            sys.set_env(k, v);
        }
        (Ctx::load(sys.clone(), t.path()).unwrap(), sys)
    }

    fn cloned(t: &TempDir, owner_repo: &str) {
        fs::create_dir_all(t.path().join("src").join(owner_repo).join(".git")).unwrap();
    }

    #[test]
    fn a_guest_is_cloned_and_registered_for_this_run_only() {
        let t = TempDir::new().unwrap();
        let (mut c, sys) = ctx(&t, &[]);
        let before = fs::read_to_string(&c.paths.config).unwrap();
        let path = t.path().join("src/other/x");
        sys.answer(
            "git",
            &[
                "clone",
                "git@github.com:other/x.git",
                &path.to_string_lossy(),
            ],
            crate::fake::ok(""),
        );
        let (key, notes) = place(&mut c, "other/x", &PlaceTo::Guest).unwrap();
        // `x` is taken, so the key has the owner in it.
        assert_eq!(key, "other-x");
        let r = c.registry.repo("other-x").unwrap();
        assert_eq!(r.path, path);
        assert_eq!(r.github.as_deref(), Some("other/x"));
        assert_eq!(
            (r.mode, r.memory.as_deref()),
            (Some(Mode::Worktree), Some("none"))
        );
        assert_eq!(c.registry.guest.as_deref(), Some("other-x"));
        assert!(matches!(&notes[0], Note::Info(s) if s.starts_with("cloning other/x")));
        assert_eq!(fs::read_to_string(&c.paths.config).unwrap(), before);
    }

    #[test]
    fn a_failed_clone_is_an_error() {
        let t = TempDir::new().unwrap();
        let (mut c, sys) = ctx(&t, &[("YO_CLONE_PROTOCOL", "https")]);
        let path = t.path().join("src/other/y");
        sys.answer(
            "git",
            &[
                "clone",
                "https://github.com/other/y.git",
                &path.to_string_lossy(),
            ],
            failed(128),
        );
        assert!(matches!(
            place(&mut c, "other/y", &PlaceTo::Guest),
            Err(Error::Failed(m)) if m.starts_with("couldn't clone https://github.com/other/y.git")
        ));
        assert!(c.registry.repo("y").is_none());
    }

    #[test]
    fn the_owners_profile_wins_and_saves_nothing() {
        let t = TempDir::new().unwrap();
        let (mut c, _) = ctx(&t, &[]);
        cloned(&t, "acme/api");
        let before = fs::read_to_string(&c.paths.config).unwrap();
        let (key, notes) = place(&mut c, "acme/api", &PlaceTo::Guest).unwrap();
        assert_eq!(key, "api");
        assert_eq!(
            c.registry.repo("api").unwrap().profile.as_deref(),
            Some("work")
        );
        assert_eq!(c.registry.guest, None);
        assert!(matches!(&notes[0], Note::Info(s) if s.contains("belongs to profile 'work'")));
        assert_eq!(fs::read_to_string(&c.paths.config).unwrap(), before);
    }

    #[test]
    fn a_picked_profile_is_saved_to_the_config() {
        let t = TempDir::new().unwrap();
        let (mut c, sys) = ctx(&t, &[]);
        cloned(&t, "other/z");
        place(&mut c, "other/z", &PlaceTo::Profile("p".into())).unwrap();
        let again = Ctx::load(sys.clone(), t.path()).unwrap();
        let r = again.registry.repo("z").unwrap();
        assert_eq!(r.profile.as_deref(), Some("p"));
        assert_eq!(r.github.as_deref(), Some("other/z"));
        assert_eq!(r.path, t.path().join("src/other/z"));
        assert!(
            fs::read_to_string(&c.paths.config)
                .unwrap()
                .contains("\"path\": \"~/src/other/z\"")
        );
    }

    #[test]
    fn a_new_profile_claims_the_owner() {
        let t = TempDir::new().unwrap();
        let (mut c, sys) = ctx(&t, &[]);
        cloned(&t, "neo/w");
        assert_eq!(choices(&c, "neo/w").new_profile.as_deref(), Some("neo"));
        // A name that's taken isn't offered.
        assert_eq!(choices(&c, "p/w").new_profile, None);
        place(&mut c, "neo/w", &PlaceTo::NewProfile("neo".into())).unwrap();
        assert_eq!(
            c.registry.repo("w").unwrap().profile.as_deref(),
            Some("neo")
        );
        let again = Ctx::load(sys.clone(), t.path()).unwrap();
        assert_eq!(again.registry.profile_for_owner("neo").unwrap().name, "neo");
    }

    #[test]
    fn refusals() {
        let t = TempDir::new().unwrap();
        let (mut c, _) = ctx(&t, &[]);
        fs::create_dir_all(t.path().join("src/other/notgit")).unwrap();
        assert!(matches!(
            place(&mut c, "other/notgit", &PlaceTo::Guest),
            Err(Error::Refused(m)) if m.contains("isn't a git clone")
        ));
        for bad in ["a/..", "../a", "a", "a/b c", "-a/b"] {
            assert!(place(&mut c, bad, &PlaceTo::Guest).is_err(), "{bad}");
        }
        assert!(PlaceTo::parse(&c, "nope").is_err());
        assert_eq!(PlaceTo::parse(&c, "guest").unwrap(), PlaceTo::Guest);
        assert_eq!(
            PlaceTo::parse(&c, "p").unwrap(),
            PlaceTo::Profile("p".into())
        );
    }

    #[test]
    fn a_guest_profile_takes_guests() {
        let t = TempDir::new().unwrap();
        let (mut c, _) = ctx(&t, &[("YO_ADHOC_PROFILE", "u")]);
        cloned(&t, "other/v");
        place(&mut c, "other/v", &PlaceTo::Guest).unwrap();
        let r = c.registry.repo("v").unwrap();
        assert_eq!(
            (r.profile.as_deref(), r.memory.as_deref()),
            (Some("u"), None)
        );
        assert_eq!(c.registry.guest.as_deref(), Some("v"));
    }
}
