//! The config resolved: the repos, profiles and agents yo knows, and the
//! global settings with the environment applied. Built the way bash's
//! loader builds it, in the same order, with the same checks.
//!
//! Precedence lives in one place, [`Registry::layered`]: a flag, then the
//! repo's own setting, then its profile's, then the global one. A global
//! setting's environment variable beats the file ([`Defaults`]).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{Config, RepoCfg, Text};
use crate::error::{Error, Result};
use crate::paths::{expand_home, strip_trailing_slash};
use crate::sys::Sys;
use crate::{git, github};

/// Where a session runs: in a new worktree, or in place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Worktree,
    Here,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "worktree" => Some(Mode::Worktree),
            "here" => Some(Mode::Here),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Worktree => "worktree",
            Mode::Here => "here",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repo {
    pub key: String,
    /// The primary clone, without a trailing slash.
    pub path: PathBuf,
    pub base: Option<String>,
    pub github: Option<String>,
    pub mcp: Vec<String>,
    pub mirror: Vec<String>,
    /// `None`: the default, `[key] ` for every repo but the default one.
    pub title_prefix: Option<String>,
    pub memory: Option<String>,
    pub branch_prefix: Option<String>,
    pub mode: Option<Mode>,
    pub profile: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub name: String,
    pub mode: Option<Mode>,
    pub memory: Option<String>,
    pub branch_prefix: Option<String>,
    pub claude: Option<String>,
    pub settings: Option<PathBuf>,
    pub dir: Option<PathBuf>,
    pub window_group: Option<String>,
    pub owners: Vec<String>,
    /// Whether yo pre-trusts its worktrees.
    pub trust: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agent {
    pub name: String,
    pub repo: String,
    pub title: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowMode {
    Tab,
    Window,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Auto,
    Split,
    Workgroup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloneProtocol {
    Ssh,
    Https,
}

/// The global settings: each one's environment variable if it's set, else
/// the config's `defaults`, else yo's default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Defaults {
    /// `YO_BRANCH_PREFIX`; empty is no prefix.
    pub branch_prefix: String,
    /// `YO_MODE`
    pub mode: Mode,
    /// `YO_CLAUDE_CMD`
    pub claude: String,
    /// `YO_MEMORIES`: the shared memory root. `None` when set to nothing,
    /// which turns shared memory off.
    pub memories: Option<PathBuf>,
    /// `YO_MEMORY_STORE`: the store inside it.
    pub memory: String,
    /// `YO_CLAUDE_SETTINGS`
    pub settings: Option<PathBuf>,
    /// `YO_LAYOUT`
    pub layout: Layout,
    /// `YO_WINDOW_MODE`
    pub window_mode: WindowMode,
    /// `YO_WINDOW_GROUP`
    pub window_group: String,
    /// `YO_WIN_WIDTH`, `YO_WIN_HEIGHT`
    pub win_width: String,
    pub win_height: String,
    /// `YO_ITERM_PROFILE`
    pub iterm_profile: String,
    /// `YO_REVIEW_CMD`
    pub review_cmd: String,
    /// `YO_DEFAULT_REPO`
    pub default_repo: Option<String>,
    /// `YO_CLONE_ROOT`
    pub clone_root: PathBuf,
    /// `YO_CLONE_PROTOCOL`
    pub clone_protocol: CloneProtocol,
    /// `YO_ADHOC_PROFILE` (`guestProfile` in the file)
    pub guest_profile: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Registry {
    /// The config file it came from; `None` when there was none.
    pub config: Option<PathBuf>,
    pub defaults: Defaults,
    repos: Vec<Repo>,
    profiles: Vec<Profile>,
    agents: Vec<Agent>,
    /// The repo registered from the cwd in single-repo mode (no repos
    /// configured), if any.
    pub autoregistered: Option<String>,
}

impl Registry {
    /// The registry from the config at `config_path` (none there: single-repo
    /// mode), with the cwd's clone registered when nothing else is.
    pub fn load(sys: &dyn Sys, config_path: &Path, cwd: &Path) -> Result<Registry> {
        let cfg = Config::load(config_path)?;
        let found = cfg.is_some();
        let mut reg = Registry::build(sys, &cfg.unwrap_or_default())?;
        if found {
            reg.config = Some(config_path.to_path_buf());
        }
        if reg.repos.is_empty()
            && let Some(clone) = git::primary_clone(sys, cwd)
        {
            let key = clone
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            check_key("yo_repo: key", &key)?;
            reg.repos.push(Repo::bare(key.clone(), clone));
            reg.autoregistered = Some(key);
        }
        Ok(reg)
    }

    /// The registry a parsed config describes, in bash's order: profiles,
    /// then listed repos (top-level, then each profile's), then scanned
    /// ones (top-level `reposIn`, then each profile's `reposIn` and owners),
    /// then agents.
    pub fn build(sys: &dyn Sys, cfg: &Config) -> Result<Registry> {
        let home = sys.home();
        let mut b = Builder {
            home: &home,
            repos: Vec::new(),
            keys: HashSet::new(),
            paths: HashSet::new(),
        };

        let mut profiles = Vec::new();
        for (name, p) in cfg.profiles.iter() {
            check_key("yo_profile: name", name)?;
            let mode = match &p.mode {
                Some(m) => Some(Mode::parse(m.as_str()).ok_or_else(|| {
                    Error::Setting(format!(
                        "yo_profile {name}: --mode must be 'here' or 'worktree' (got: '{}')",
                        m.as_str()
                    ))
                })?),
                None => None,
            };
            profiles.push(Profile {
                name: name.to_string(),
                mode,
                memory: nonempty(&p.memory),
                branch_prefix: nonempty(&p.branch_prefix),
                claude: nonempty(&p.claude),
                settings: nonempty(&p.settings).map(|s| expand_home(&home, &s)),
                dir: nonempty(&p.dir).map(|s| expand_home(&home, strip_trailing_slash(&s))),
                window_group: nonempty(&p.window_group),
                owners: p.owners.clone().map(|w| w.0).unwrap_or_default(),
                trust: p.trust != Some(serde_json::Value::Bool(false)),
            });
        }

        for (key, r) in cfg.repos.iter() {
            b.listed(key, r, None, &format!("repos.{key}"))?;
        }
        for (name, p) in cfg.profiles.iter() {
            for (key, r) in p.repos.iter() {
                b.listed(key, r, Some(name), &format!("profiles.{name}.repos.{key}"))?;
            }
        }

        let clone_root = expand_home(
            &home,
            &sys.env_nonempty("YO_CLONE_ROOT")
                .or_else(|| nonempty(&cfg.defaults.clone_root))
                .unwrap_or_else(|| "~/dev/src/github.com".to_string()),
        );
        for dir in &cfg.repos_in {
            b.scan(&expand_home(&home, dir.as_str()), None, None);
        }
        for (name, p) in cfg.profiles.iter() {
            for dir in &p.repos_in {
                b.scan(&expand_home(&home, dir.as_str()), Some(name), None);
            }
            for owner in p.owners.iter().flat_map(|w| &w.0) {
                b.scan(&clone_root.join(owner), Some(name), Some(owner));
            }
        }

        let mut agents: Vec<Agent> = Vec::new();
        let all_agents = cfg
            .agents
            .iter()
            .chain(cfg.profiles.iter().flat_map(|(_, p)| p.agents.iter()));
        for (name, a) in all_agents {
            check_key("yo_agent: name", name)?;
            if a.repo.as_str().is_empty() {
                return Err(Error::Setting(format!(
                    "yo_agent {name}: --repo is required"
                )));
            }
            if agents.iter().any(|x| x.name == name) {
                return Err(Error::Setting(format!(
                    "yo_agent: '{name}' is already registered"
                )));
            }
            agents.push(Agent {
                name: name.to_string(),
                repo: a.repo.0.clone(),
                title: nonempty(&a.title),
            });
        }

        let defaults = Defaults::resolve(sys, cfg, &profiles)?;
        Ok(Registry {
            config: None,
            defaults,
            repos: b.repos,
            profiles,
            agents,
            autoregistered: None,
        })
    }

    pub fn repos(&self) -> &[Repo] {
        &self.repos
    }

    pub fn profiles(&self) -> &[Profile] {
        &self.profiles
    }

    pub fn agents(&self) -> &[Agent] {
        &self.agents
    }

    pub fn repo(&self, key: &str) -> Option<&Repo> {
        self.repos.iter().find(|r| r.key == key)
    }

    pub fn profile(&self, name: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.name == name)
    }

    pub fn agent(&self, name: &str) -> Option<&Agent> {
        self.agents.iter().find(|a| a.name == name)
    }

    /// The repo whose primary clone is `dir`.
    pub fn repo_at(&self, dir: &Path) -> Option<&Repo> {
        let want = dir.to_string_lossy();
        let want = strip_trailing_slash(&want);
        self.repos.iter().find(|r| {
            let p = r.path.to_string_lossy();
            strip_trailing_slash(&p) == want
        })
    }

    /// The repo bare `yo` falls back to: `YO_DEFAULT_REPO` if it names one,
    /// else the first registered.
    pub fn default_repo(&self) -> Option<&Repo> {
        self.defaults
            .default_repo
            .as_deref()
            .and_then(|k| self.repo(k))
            .or_else(|| self.repos.first())
    }

    /// The repos in profile `name`, in registration order.
    pub fn repos_of_profile(&self, name: &str) -> impl Iterator<Item = &Repo> {
        self.repos
            .iter()
            .filter(move |r| r.profile.as_deref() == Some(name))
    }

    /// The profile that claims GitHub owner `owner` (case-insensitive).
    pub fn profile_for_owner(&self, owner: &str) -> Option<&Profile> {
        self.profiles
            .iter()
            .find(|p| p.owners.iter().any(|o| o.eq_ignore_ascii_case(owner)))
    }

    /// The profile a session runs under: the one picked (`--profile`, or
    /// `yo <profile>`), else its repo's own.
    pub fn profile_of(&self, repo: Option<&Repo>, picked: Option<&str>) -> Option<&Profile> {
        match picked {
            Some(p) => self.profile(p),
            None => repo
                .and_then(|r| r.profile.as_deref())
                .and_then(|p| self.profile(p)),
        }
    }

    /// The one place precedence lives: `flag`, then the repo's own value,
    /// then its profile's (see [`Registry::profile_of`]), then `default`.
    pub fn layered<T>(
        &self,
        flag: Option<T>,
        repo: Option<&Repo>,
        picked: Option<&str>,
        of_repo: impl Fn(&Repo) -> Option<T>,
        of_profile: impl Fn(&Profile) -> Option<T>,
        default: T,
    ) -> T {
        flag.or_else(|| repo.and_then(&of_repo))
            .or_else(|| self.profile_of(repo, picked).and_then(&of_profile))
            .unwrap_or(default)
    }

    pub fn mode(&self, flag: Option<Mode>, repo: Option<&Repo>, picked: Option<&str>) -> Mode {
        self.layered(
            flag,
            repo,
            picked,
            |r| r.mode,
            |p| p.mode,
            self.defaults.mode,
        )
    }

    /// The prefix for branches yo creates.
    pub fn branch_prefix(&self, repo: Option<&Repo>, picked: Option<&str>) -> String {
        self.layered(
            None,
            repo,
            picked,
            |r| r.branch_prefix.clone(),
            |p| p.branch_prefix.clone(),
            self.defaults.branch_prefix.clone(),
        )
    }

    /// The command for the claude harness. Repos don't set it.
    pub fn claude_cmd(&self, repo: Option<&Repo>, picked: Option<&str>) -> String {
        self.layered(
            None,
            repo,
            picked,
            |_| None,
            |p| p.claude.clone(),
            self.defaults.claude.clone(),
        )
    }

    /// The shared window a session joins. Repos don't set it.
    pub fn window_group(&self, repo: Option<&Repo>, picked: Option<&str>) -> String {
        self.layered(
            None,
            repo,
            picked,
            |_| None,
            |p| p.window_group.clone(),
            self.defaults.window_group.clone(),
        )
    }

    /// The auto-memory directory: one store shared by every session unless
    /// a repo (or its profile) names its own. `None` (Claude keeps its own
    /// per-project memory) when the store is `none` or `YO_MEMORIES` is set
    /// to nothing.
    pub fn memory_dir(&self, repo: Option<&Repo>, picked: Option<&str>) -> Option<PathBuf> {
        let root = self.defaults.memories.as_ref()?;
        let store = self.layered(
            None,
            repo,
            picked,
            |r| r.memory.clone(),
            |p| p.memory.clone(),
            self.defaults.memory.clone(),
        );
        if store == "none" {
            None
        } else if store.starts_with('/') {
            Some(PathBuf::from(store))
        } else {
            let root = root.to_string_lossy();
            Some(PathBuf::from(format!(
                "{}/{store}",
                root.trim_end_matches('/')
            )))
        }
    }

    /// The window-title prefix: the repo's own, else `[key] ` for every repo
    /// but the default one.
    pub fn title_prefix(&self, repo: &Repo) -> String {
        match &repo.title_prefix {
            Some(p) => p.clone(),
            None if self.default_repo().map(|d| &d.key) == Some(&repo.key) => String::new(),
            None => format!("[{}] ", repo.key),
        }
    }

    /// `owner/repo`: the configured `github`, else the clone's `upstream`
    /// or `origin` remote, else a `.../github.com/<owner>/<repo>` path (a
    /// repo configured before it's cloned here).
    pub fn github_for(&self, sys: &dyn Sys, repo: &Repo) -> Option<String> {
        if let Some(gh) = &repo.github {
            return Some(gh.clone());
        }
        for r in ["upstream", "origin"] {
            if let Some(gh) =
                git::remote_url(sys, &repo.path, r).and_then(|u| github::owner_repo_from_url(&u))
            {
                return Some(gh);
            }
        }
        github::owner_repo_from_path(&repo.path.to_string_lossy())
    }

    /// The branch sessions start from: the configured `base`, else the
    /// clone's default branch.
    pub fn base_ref(&self, sys: &dyn Sys, repo: &Repo) -> String {
        repo.base
            .clone()
            .unwrap_or_else(|| git::default_branch(sys, &repo.path))
    }

    /// The repo a GitHub URL (a remote, a PR, an issue) belongs to.
    pub fn repo_for_url(&self, sys: &dyn Sys, url: &str) -> Option<&Repo> {
        let want = github::owner_repo_from_url(url)?;
        self.repos
            .iter()
            .find(|r| self.github_for(sys, r).as_deref() == Some(want.as_str()))
    }

    /// The repo `dir` belongs to: by any of its remotes, else by being (a
    /// worktree of) a registered clone.
    pub fn detect_repo(&self, sys: &dyn Sys, dir: &Path) -> Option<&Repo> {
        for r in git::remotes(sys, dir) {
            if let Some(repo) =
                git::remote_url(sys, dir, &r).and_then(|u| self.repo_for_url(sys, &u))
            {
                return Some(repo);
            }
        }
        let clone = git::primary_clone(sys, dir)?;
        self.repo_at(&clone).or_else(|| {
            let real = fs::canonicalize(&clone).ok()?;
            self.repo_at(&real)
        })
    }
}

impl Repo {
    fn bare(key: String, path: PathBuf) -> Repo {
        Repo {
            key,
            path,
            base: None,
            github: None,
            mcp: Vec::new(),
            mirror: Vec::new(),
            title_prefix: None,
            memory: None,
            branch_prefix: None,
            mode: None,
            profile: None,
        }
    }
}

struct Builder<'a> {
    home: &'a Path,
    repos: Vec<Repo>,
    keys: HashSet<String>,
    /// Real paths of every clone registered, so a scan skips listed ones.
    paths: HashSet<PathBuf>,
}

impl Builder<'_> {
    fn listed(&mut self, key: &str, r: &RepoCfg, profile: Option<&str>, at: &str) -> Result<()> {
        if self.keys.contains(key) {
            return Err(Error::Setting(format!(
                "{at}: repo '{key}' is listed twice"
            )));
        }
        check_key("yo_repo: key", key)?;
        let path = expand_home(self.home, strip_trailing_slash(r.path.as_str()));
        if path.as_os_str().is_empty() {
            return Err(Error::Setting(format!("yo_repo {key}: --path is required")));
        }
        let mode = match &r.mode {
            Some(m) => Some(Mode::parse(m.as_str()).ok_or_else(|| {
                Error::Setting(format!(
                    "yo_repo {key}: --mode must be 'here' or 'worktree' (got: '{}')",
                    m.as_str()
                ))
            })?),
            None => None,
        };
        self.keys.insert(key.to_string());
        self.paths.insert(real(&path));
        self.repos.push(Repo {
            key: key.to_string(),
            path,
            base: nonempty(&r.base),
            github: nonempty(&r.github),
            mcp: r.mcp.clone().map(|w| w.0).unwrap_or_default(),
            mirror: r.mirror.clone().map(|w| w.0).unwrap_or_default(),
            title_prefix: r.title_prefix.as_ref().map(|t| t.0.clone()),
            memory: nonempty(&r.memory),
            branch_prefix: nonempty(&r.branch_prefix),
            mode,
            profile: profile.map(str::to_string),
        });
        Ok(())
    }

    /// The clones directly under `dir`, keyed by directory name (or
    /// `owner-name` when the name is taken and there's an owner). Clones
    /// already registered, and names that aren't valid keys, are skipped.
    fn scan(&mut self, dir: &Path, profile: Option<&str>, owner: Option<&str>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        let mut names: Vec<String> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        for n in names {
            let p = dir.join(&n);
            if !p.join(".git").exists() || self.paths.contains(&real(&p)) {
                continue;
            }
            let mut key = n.clone();
            if self.keys.contains(&key)
                && let Some(o) = owner
            {
                key = format!("{o}-{n}");
            }
            if self.keys.contains(&key) || !valid_key(&key) {
                continue;
            }
            self.keys.insert(key.clone());
            self.paths.insert(real(&p));
            let mut repo = Repo::bare(key, p);
            repo.github = owner.map(|o| format!("{o}/{n}"));
            repo.profile = profile.map(str::to_string);
            self.repos.push(repo);
        }
    }
}

impl Defaults {
    fn resolve(sys: &dyn Sys, cfg: &Config, profiles: &[Profile]) -> Result<Defaults> {
        let d = &cfg.defaults;
        let home = sys.home();
        // The environment beats the file; a variable set to nothing still
        // counts as set (bash's `${X+x}`).
        let get = |var: &str, file: &Option<Text>| -> Option<String> {
            sys.env(var).or_else(|| file.as_ref().map(|t| t.0.clone()))
        };
        // A path from the file has its ~/ expanded; one from the
        // environment already met a shell.
        let get_path = |var: &str, file: &Option<Text>| -> Option<String> {
            sys.env(var).or_else(|| {
                file.as_ref().map(|t| {
                    if t.0.is_empty() {
                        String::new()
                    } else {
                        expand_home(&home, &t.0).to_string_lossy().into_owned()
                    }
                })
            })
        };
        let or = |v: Option<String>, dflt: &str| -> String {
            v.filter(|s| !s.is_empty())
                .unwrap_or_else(|| dflt.to_string())
        };

        let window_mode = match or(get("YO_WINDOW_MODE", &d.window_mode), "tab").as_str() {
            "tab" => WindowMode::Tab,
            "window" => WindowMode::Window,
            other => return Err(bad("YO_WINDOW_MODE", "'tab' or 'window'", other)),
        };
        let clone_root = expand_home(
            &home,
            &or(
                get_path("YO_CLONE_ROOT", &d.clone_root),
                &home.join("dev/src/github.com").to_string_lossy(),
            ),
        );
        let clone_protocol = match or(get("YO_CLONE_PROTOCOL", &d.clone_protocol), "ssh").as_str() {
            "ssh" => CloneProtocol::Ssh,
            "https" => CloneProtocol::Https,
            other => return Err(bad("YO_CLONE_PROTOCOL", "'ssh' or 'https'", other)),
        };
        let guest_profile = get("YO_ADHOC_PROFILE", &d.guest_profile).filter(|s| !s.is_empty());
        if let Some(g) = &guest_profile
            && !profiles.iter().any(|p| &p.name == g)
        {
            return Err(Error::Setting(format!(
                "YO_ADHOC_PROFILE: no yo_profile '{g}' is defined"
            )));
        }
        let mode_s = or(get("YO_MODE", &d.mode), "worktree");
        let mode =
            Mode::parse(&mode_s).ok_or_else(|| bad("YO_MODE", "'here' or 'worktree'", &mode_s))?;
        let layout = match or(get("YO_LAYOUT", &d.layout), "auto").as_str() {
            "auto" => Layout::Auto,
            "split" => Layout::Split,
            "workgroup" => Layout::Workgroup,
            other => return Err(bad("YO_LAYOUT", "'auto', 'split' or 'workgroup'", other)),
        };
        // Unset: ~/.claude/memories. Set to nothing: off.
        let memories = match get_path("YO_MEMORIES", &d.memories) {
            None => Some(home.join(".claude/memories")),
            Some(s) if s.is_empty() => None,
            Some(s) => Some(PathBuf::from(s)),
        };

        Ok(Defaults {
            branch_prefix: get("YO_BRANCH_PREFIX", &d.branch_prefix).unwrap_or_default(),
            mode,
            claude: or(
                get("YO_CLAUDE_CMD", &d.claude),
                "claude --permission-mode auto",
            ),
            memories,
            memory: or(get("YO_MEMORY_STORE", &d.memory), "yo"),
            settings: get_path("YO_CLAUDE_SETTINGS", &d.settings)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from),
            layout,
            window_mode,
            window_group: or(get("YO_WINDOW_GROUP", &d.window_group), "yo"),
            win_width: or(get("YO_WIN_WIDTH", &d.win_width), "1403"),
            win_height: or(get("YO_WIN_HEIGHT", &d.win_height), "824"),
            iterm_profile: or(get("YO_ITERM_PROFILE", &d.iterm_profile), "worksesh"),
            review_cmd: or(get("YO_REVIEW_CMD", &d.review_cmd), "/code-review"),
            default_repo: get("YO_DEFAULT_REPO", &d.default_repo).filter(|s| !s.is_empty()),
            clone_root,
            clone_protocol,
            guest_profile,
        })
    }
}

fn bad(var: &str, want: &str, got: &str) -> Error {
    Error::Setting(format!("{var} must be {want} (got: '{got}')"))
}

/// A repo key, profile or agent name: letters, digits, `.`, `_`, `-`, and
/// not only dots (`.` and `..` would make paths that climb out).
pub fn valid_key(k: &str) -> bool {
    !k.is_empty()
        && !k.chars().all(|c| c == '.')
        && k.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn check_key(what: &str, k: &str) -> Result<()> {
    if valid_key(k) {
        Ok(())
    } else {
        Err(Error::Setting(format!(
            "{what} may only contain letters, digits, . _ - (got: '{k}')"
        )))
    }
}

fn nonempty(t: &Option<Text>) -> Option<String> {
    t.as_ref().map(|t| t.0.clone()).filter(|s| !s.is_empty())
}

fn real(p: &Path) -> PathBuf {
    fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{FakeSys, TempDir};

    fn cfg(json: &str) -> Config {
        serde_json::from_str(json).unwrap()
    }

    fn reg(sys: &FakeSys, json: &str) -> Registry {
        Registry::build(sys, &cfg(json)).unwrap()
    }

    fn err(sys: &FakeSys, json: &str) -> String {
        Registry::build(sys, &cfg(json)).unwrap_err().to_string()
    }

    fn keys(r: &Registry) -> Vec<&str> {
        r.repos().iter().map(|r| r.key.as_str()).collect()
    }

    #[test]
    fn defaults_with_nothing_configured() {
        let sys = FakeSys::new("/h");
        let d = reg(&sys, "{}").defaults;
        assert_eq!(d.mode, Mode::Worktree);
        assert_eq!(d.claude, "claude --permission-mode auto");
        assert_eq!(d.branch_prefix, "");
        assert_eq!(d.memories, Some(PathBuf::from("/h/.claude/memories")));
        assert_eq!(d.memory, "yo");
        assert_eq!(d.window_mode, WindowMode::Tab);
        assert_eq!(d.window_group, "yo");
        assert_eq!(
            (d.win_width.as_str(), d.win_height.as_str()),
            ("1403", "824")
        );
        assert_eq!(d.review_cmd, "/code-review");
        assert_eq!(d.clone_root, PathBuf::from("/h/dev/src/github.com"));
        assert_eq!(d.clone_protocol, CloneProtocol::Ssh);
        assert_eq!(d.layout, Layout::Auto);
    }

    #[test]
    fn environment_beats_the_file() {
        let sys = FakeSys::new("/h");
        let json = r#"{"defaults": {"mode": "here", "claude": "c1", "windowGroup": "g"}}"#;
        let d = reg(&sys, json).defaults;
        assert_eq!(
            (d.mode, d.claude.as_str(), d.window_group.as_str()),
            (Mode::Here, "c1", "g")
        );
        sys.set_env("YO_MODE", "worktree");
        sys.set_env("YO_CLAUDE_CMD", "c2");
        let d = reg(&sys, json).defaults;
        assert_eq!((d.mode, d.claude.as_str()), (Mode::Worktree, "c2"));
    }

    #[test]
    fn a_variable_set_to_nothing() {
        let sys = FakeSys::new("/h");
        let json = r#"{"defaults": {"branchPrefix": "me", "claude": "c1"}}"#;
        sys.set_env("YO_BRANCH_PREFIX", "");
        sys.set_env("YO_CLAUDE_CMD", "");
        sys.set_env("YO_MEMORIES", "");
        let d = reg(&sys, json).defaults;
        // Kept as empty where bash uses ${X-...}, the default where ${X:-...}.
        assert_eq!(d.branch_prefix, "");
        assert_eq!(d.claude, "claude --permission-mode auto");
        assert_eq!(d.memories, None);
    }

    #[test]
    fn file_paths_expand_home() {
        let sys = FakeSys::new("/h");
        let d = reg(
            &sys,
            r#"{"defaults": {"cloneRoot": "~/src", "memories": "~/mem", "settings": "~/s.json"}}"#,
        )
        .defaults;
        assert_eq!(d.clone_root, PathBuf::from("/h/src"));
        assert_eq!(d.memories, Some(PathBuf::from("/h/mem")));
        assert_eq!(d.settings, Some(PathBuf::from("/h/s.json")));
    }

    #[test]
    fn bad_values() {
        let sys = FakeSys::new("/h");
        assert_eq!(
            err(&sys, r#"{"defaults": {"mode": "there"}}"#),
            "YO_MODE must be 'here' or 'worktree' (got: 'there')"
        );
        assert_eq!(
            err(&sys, r#"{"defaults": {"windowMode": "pane"}}"#),
            "YO_WINDOW_MODE must be 'tab' or 'window' (got: 'pane')"
        );
        assert_eq!(
            err(&sys, r#"{"defaults": {"guestProfile": "nope"}}"#),
            "YO_ADHOC_PROFILE: no yo_profile 'nope' is defined"
        );
        assert_eq!(
            err(&sys, r#"{"repos": {"a": {"path": "/a", "mode": "x"}}}"#),
            "yo_repo a: --mode must be 'here' or 'worktree' (got: 'x')"
        );
        assert_eq!(
            err(&sys, r#"{"repos": {"a b": {"path": "/a"}}}"#),
            "yo_repo: key may only contain letters, digits, . _ - (got: 'a b')"
        );
        assert_eq!(
            err(
                &sys,
                r#"{"repos": {"a": {"path": "/a"}}, "profiles": {"p": {"repos": {"a": {"path": "/b"}}}}}"#
            ),
            "profiles.p.repos.a: repo 'a' is listed twice"
        );
        assert_eq!(
            err(
                &sys,
                r#"{"agents": {"x": {"repo": "a"}}, "profiles": {"p": {"agents": {"x": {"repo": "b"}}}}}"#
            ),
            "yo_agent: 'x' is already registered"
        );
    }

    #[test]
    fn repos_in_order_with_home_and_slashes() {
        let sys = FakeSys::new("/h");
        let r = reg(
            &sys,
            r#"{"profiles": {"p": {"repos": {"pa": {"path": "/pa"}}}},
                "repos": {"z": {"path": "~/z/"}, "a": {"path": "/a", "titlePrefix": ""}}}"#,
        );
        assert_eq!(keys(&r), ["z", "a", "pa"]);
        assert_eq!(r.repo("z").unwrap().path, PathBuf::from("/h/z"));
        assert_eq!(r.repo("pa").unwrap().profile.as_deref(), Some("p"));
        assert_eq!(r.default_repo().unwrap().key, "z");
        assert_eq!(r.title_prefix(r.repo("z").unwrap()), "");
        assert_eq!(r.title_prefix(r.repo("a").unwrap()), "");
        assert_eq!(r.title_prefix(r.repo("pa").unwrap()), "[pa] ");
        assert_eq!(r.repos_of_profile("p").count(), 1);
        assert_eq!(r.repo_at(Path::new("/h/z/")).unwrap().key, "z");
    }

    #[test]
    fn default_repo_from_the_environment() {
        let sys = FakeSys::new("/h");
        let json = r#"{"repos": {"a": {"path": "/a"}, "b": {"path": "/b"}}}"#;
        sys.set_env("YO_DEFAULT_REPO", "b");
        assert_eq!(reg(&sys, json).default_repo().unwrap().key, "b");
        sys.set_env("YO_DEFAULT_REPO", "nope");
        assert_eq!(reg(&sys, json).default_repo().unwrap().key, "a");
    }

    #[test]
    fn repos_in_and_owners_are_scanned() {
        let t = TempDir::new().unwrap();
        let home = t.path();
        for d in [
            "mine/one",
            "mine/two",
            "mine/notaclone",
            "gh/acme/one",
            "gh/acme/api",
        ] {
            fs::create_dir_all(home.join(d)).unwrap();
        }
        for d in ["mine/one", "mine/two", "gh/acme/one", "gh/acme/api"] {
            fs::create_dir_all(home.join(d).join(".git")).unwrap();
        }
        let sys = FakeSys::new(home);
        let r = reg(
            &sys,
            r#"{"defaults": {"cloneRoot": "~/gh"},
                "repos": {"two": {"path": "~/mine/two", "base": "dev"}},
                "reposIn": ["~/mine"],
                "profiles": {"work": {"owners": ["acme"]}}}"#,
        );
        // `two` is listed, so the scan leaves it; acme's `one` clashes with
        // mine and takes the owner's name.
        assert_eq!(keys(&r), ["two", "one", "api", "acme-one"]);
        let api = r.repo("api").unwrap();
        assert_eq!(api.github.as_deref(), Some("acme/api"));
        assert_eq!(api.profile.as_deref(), Some("work"));
        assert_eq!(
            r.repo("acme-one").unwrap().github.as_deref(),
            Some("acme/one")
        );
        assert_eq!(r.repo("two").unwrap().base.as_deref(), Some("dev"));
        assert_eq!(r.profile_for_owner("ACME").unwrap().name, "work");
    }

    #[test]
    fn precedence_flag_repo_profile_defaults() {
        let sys = FakeSys::new("/h");
        let r = reg(
            &sys,
            r#"{"defaults": {"branchPrefix": "d"},
                "profiles": {"p": {"mode": "here", "branchPrefix": "pp", "claude": "pc",
                                   "repos": {"a": {"path": "/a", "mode": "worktree", "branchPrefix": "ra"},
                                             "b": {"path": "/b"}}},
                             "q": {"branchPrefix": "qq"}},
                "repos": {"c": {"path": "/c"}}}"#,
        );
        let a = r.repo("a");
        let b = r.repo("b");
        let c = r.repo("c");
        // mode: flag, then repo, then profile, then defaults
        assert_eq!(r.mode(Some(Mode::Here), a, None), Mode::Here);
        assert_eq!(r.mode(None, a, None), Mode::Worktree);
        assert_eq!(r.mode(None, b, None), Mode::Here);
        assert_eq!(r.mode(None, c, None), Mode::Worktree);
        // branch prefix: repo, then profile (picked beats the repo's own), then defaults
        assert_eq!(r.branch_prefix(a, None), "ra");
        assert_eq!(r.branch_prefix(b, None), "pp");
        assert_eq!(r.branch_prefix(b, Some("q")), "qq");
        assert_eq!(r.branch_prefix(c, None), "d");
        assert_eq!(r.branch_prefix(None, Some("p")), "pp");
        // the claude command: profile, then defaults
        assert_eq!(r.claude_cmd(b, None), "pc");
        assert_eq!(r.claude_cmd(c, None), "claude --permission-mode auto");
    }

    #[test]
    fn memory_dirs() {
        let sys = FakeSys::new("/h");
        let r = reg(
            &sys,
            r#"{"profiles": {"p": {"memory": "work"}},
                "repos": {"a": {"path": "/a"}, "n": {"path": "/n", "memory": "none"},
                          "abs": {"path": "/x", "memory": "/elsewhere"}}}"#,
        );
        assert_eq!(
            r.memory_dir(r.repo("a"), None),
            Some(PathBuf::from("/h/.claude/memories/yo"))
        );
        assert_eq!(
            r.memory_dir(r.repo("a"), Some("p")),
            Some(PathBuf::from("/h/.claude/memories/work"))
        );
        assert_eq!(r.memory_dir(r.repo("n"), None), None);
        assert_eq!(
            r.memory_dir(r.repo("abs"), None),
            Some(PathBuf::from("/elsewhere"))
        );
        sys.set_env("YO_MEMORIES", "");
        let r = reg(&sys, r#"{"repos": {"a": {"path": "/a"}}}"#);
        assert_eq!(r.memory_dir(r.repo("a"), None), None);
    }

    #[test]
    fn profiles_resolved() {
        let sys = FakeSys::new("/h");
        let r = reg(
            &sys,
            r#"{"profiles": {"p": {"dir": "~/work/", "settings": "~/s.json", "owners": "a b", "trust": false},
                             "q": {"trust": true}}}"#,
        );
        let p = r.profile("p").unwrap();
        assert_eq!(p.dir, Some(PathBuf::from("/h/work")));
        assert_eq!(p.settings, Some(PathBuf::from("/h/s.json")));
        assert_eq!(p.owners, ["a", "b"]);
        assert!(!p.trust);
        assert!(r.profile("q").unwrap().trust);
    }

    #[test]
    fn single_repo_mode() {
        let sys = FakeSys::new("/h");
        sys.answer_ok(
            "git",
            &["-C", "/work/proj/sub", "rev-parse", "--show-toplevel"],
            "/work/proj\n",
        );
        sys.answer_ok(
            "git",
            &["-C", "/work/proj/sub", "rev-parse", "--git-common-dir"],
            "/work/proj/.git\n",
        );
        let r = Registry::load(
            &sys,
            Path::new("/no/config.json"),
            Path::new("/work/proj/sub"),
        )
        .unwrap();
        assert_eq!(r.config, None);
        assert_eq!(keys(&r), ["proj"]);
        assert_eq!(r.autoregistered.as_deref(), Some("proj"));
        assert_eq!(r.repo("proj").unwrap().path, PathBuf::from("/work/proj"));
        // Outside a clone, with no config: no repos, and no error.
        let r = Registry::load(&sys, Path::new("/no/config.json"), Path::new("/tmp")).unwrap();
        assert!(r.repos().is_empty());
    }

    #[test]
    fn more_bad_values() {
        let sys = FakeSys::new("/h");
        for (json, want) in [
            (
                r#"{"repos": {"a": {"path": ""}}}"#,
                "yo_repo a: --path is required",
            ),
            (
                r#"{"agents": {"x": {"repo": ""}}}"#,
                "yo_agent x: --repo is required",
            ),
            (
                r#"{"profiles": {"p": {"mode": "there"}}}"#,
                "yo_profile p: --mode must be 'here' or 'worktree' (got: 'there')",
            ),
            (
                r#"{"defaults": {"cloneProtocol": "ftp"}}"#,
                "YO_CLONE_PROTOCOL must be 'ssh' or 'https' (got: 'ftp')",
            ),
            (
                r#"{"defaults": {"layout": "tiles"}}"#,
                "YO_LAYOUT must be 'auto', 'split' or 'workgroup' (got: 'tiles')",
            ),
        ] {
            assert_eq!(err(&sys, json), want, "{json}");
        }
        // From the environment, over a good value in the file.
        sys.set_env("YO_LAYOUT", "grid");
        assert_eq!(
            err(&sys, r#"{"defaults": {"layout": "split"}}"#),
            "YO_LAYOUT must be 'auto', 'split' or 'workgroup' (got: 'grid')"
        );
    }

    #[test]
    fn keys_of_only_dots_are_refused() {
        let sys = FakeSys::new("/h");
        assert!(err(&sys, r#"{"repos": {"..": {"path": "/a"}}}"#).contains("got: '..'"));
        assert!(err(&sys, r#"{"agents": {".": {"repo": "a"}}}"#).contains("got: '.'"));
        assert!(valid_key(".dotfiles") && valid_key("a.b") && valid_key("-x"));
    }

    #[test]
    fn clone_root_from_the_environment_beats_the_file() {
        let t = TempDir::new().unwrap();
        for d in ["file/acme/api/.git", "env/acme/web/.git"] {
            fs::create_dir_all(t.path().join(d)).unwrap();
        }
        let sys = FakeSys::new(t.path());
        let json =
            r#"{"defaults": {"cloneRoot": "~/file"}, "profiles": {"w": {"owners": ["acme"]}}}"#;
        assert_eq!(keys(&reg(&sys, json)), ["api"]);
        sys.set_env("YO_CLONE_ROOT", &t.path().join("env").to_string_lossy());
        let r = reg(&sys, json);
        assert_eq!(keys(&r), ["web"]);
        assert_eq!(r.defaults.clone_root, t.path().join("env"));
    }

    #[test]
    fn detect_repo_by_remote_then_clone() {
        let t = TempDir::new().unwrap();
        let clone = t.path().join("src/api");
        fs::create_dir_all(clone.join(".git")).unwrap();
        let link = t.path().join("link");
        std::os::unix::fs::symlink(t.path().join("src"), &link).unwrap();
        let sys = FakeSys::new(t.path());
        let json = format!(
            r#"{{"repos": {{"api": {{"path": "{}"}}, "web": {{"path": "/web", "github": "acme/web"}}}}}}"#,
            clone.display()
        );
        let r = reg(&sys, &json);
        let git = |dir: &str, args: &[&str], out: &str| {
            let mut a = vec!["-C", dir];
            a.extend_from_slice(args);
            sys.answer_ok("git", &a, out);
        };

        // A remote names a registered repo, whatever directory it's in.
        git("/w1", &["remote"], "fork\nupstream\n");
        git(
            "/w1",
            &["remote", "get-url", "fork"],
            "git@github.com:me/web.git",
        );
        git(
            "/w1",
            &["remote", "get-url", "upstream"],
            "git@github.com:acme/web.git",
        );
        assert_eq!(r.detect_repo(&sys, Path::new("/w1")).unwrap().key, "web");

        // No remote matches: a worktree of a registered clone.
        git("/w2", &["remote"], "origin\n");
        git(
            "/w2",
            &["remote", "get-url", "origin"],
            "git@github.com:someone/else.git",
        );
        git("/w2", &["rev-parse", "--show-toplevel"], "/w2");
        git(
            "/w2",
            &["rev-parse", "--git-common-dir"],
            &format!("{}/.git", clone.display()),
        );
        assert_eq!(r.detect_repo(&sys, Path::new("/w2")).unwrap().key, "api");

        // The clone reached through a symlink: found by its real path.
        git(
            "/w3",
            &["rev-parse", "--show-toplevel"],
            &link.join("api").to_string_lossy(),
        );
        git("/w3", &["rev-parse", "--git-common-dir"], ".git");
        assert_eq!(r.detect_repo(&sys, Path::new("/w3")).unwrap().key, "api");

        // Not in a clone at all, or in an unregistered one.
        assert_eq!(r.detect_repo(&sys, Path::new("/nowhere")), None);
        git("/w4", &["rev-parse", "--show-toplevel"], "/other");
        git("/w4", &["rev-parse", "--git-common-dir"], ".git");
        assert_eq!(r.detect_repo(&sys, Path::new("/w4")), None);
    }

    #[test]
    fn github_and_base_from_git() {
        let sys = FakeSys::new("/h");
        let r = reg(
            &sys,
            r#"{"repos": {"a": {"path": "/a"}, "b": {"path": "/h/dev/src/github.com/o/b"},
                          "c": {"path": "/c", "github": "o/c", "base": "dev"}}}"#,
        );
        sys.answer_ok(
            "git",
            &["-C", "/a", "remote", "get-url", "upstream"],
            "git@github.com:real/a.git\n",
        );
        sys.answer_ok(
            "git",
            &["-C", "/a", "remote", "get-url", "origin"],
            "git@github.com:fork/a.git\n",
        );
        let a = r.repo("a").unwrap();
        assert_eq!(r.github_for(&sys, a).as_deref(), Some("real/a"));
        assert_eq!(
            r.github_for(&sys, r.repo("b").unwrap()).as_deref(),
            Some("o/b")
        );
        assert_eq!(
            r.github_for(&sys, r.repo("c").unwrap()).as_deref(),
            Some("o/c")
        );
        assert_eq!(
            r.repo_for_url(&sys, "https://github.com/real/a/pull/1")
                .unwrap()
                .key,
            "a"
        );
        assert_eq!(r.base_ref(&sys, r.repo("c").unwrap()), "dev");
        assert_eq!(r.base_ref(&sys, a), "main");
    }
}
