//! Creating a session: which repo, worktree or in place, the branch, the
//! title, codename and first message, the ledger record and the launch.
//! bash's `cmd_up` and `create_session_worktree`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::github::{self, GhKind, GhRef};
use crate::harness::{Harness, IdPolicy, PrepareAsk, SessionSpec};
use crate::ledger::{Entry, Key, Record};
use crate::plan::{Action, BranchSpec, Note, Plan};
use crate::registry::{Mode, Repo};
use crate::runner::{LaunchSpec, RunnerKind, WindowChoice};
use crate::{Ctx, Error, Result, git, naming};

/// `yo [flags] [REPO|PROFILE] [INPUT] [EXTRA]`, as parsed.
#[derive(Clone, Debug, Default)]
pub struct CreateRequest {
    pub cwd: PathBuf,
    pub repo: Option<String>,
    pub profile: Option<String>,
    /// `--here` or `--worktree`.
    pub mode: Option<Mode>,
    pub window: WindowChoice,
    pub add_to: Option<String>,
    pub title: Option<String>,
    pub from: Option<String>,
    pub model: Option<String>,
    pub agent: Option<String>,
    /// `--harness`
    pub harness: Option<String>,
    /// The positional arguments.
    pub args: Vec<String>,
}

/// What INPUT asks for.
enum Input {
    /// Nothing, or `-`.
    Unnamed,
    Url(GhRef),
    /// A session name, or an existing branch.
    Name(String),
}

pub fn plan_create(ctx: &Ctx, req: &CreateRequest) -> Result<Plan> {
    let reg = &ctx.registry;
    let sys = &*ctx.sys;

    let repo_list = || {
        reg.repos()
            .iter()
            .map(|r| r.key.as_str())
            .collect::<Vec<_>>()
            .join(" | ")
    };
    let Parsed {
        repo_override,
        picked,
        input,
        extra,
    } = parse(ctx, req)?;

    let agent = req.agent.as_deref();
    let agent_cfg = agent.and_then(|a| reg.agent(a));
    let agent_display =
        agent.map(|a| naming::agent_display(a, agent_cfg.and_then(|c| c.title.as_deref())));
    let agent_repo = agent_cfg.map(|c| c.repo.clone());
    let here_flag = req.mode == Some(Mode::Here);

    // Which repo. One that was named (--repo, positionally, by a URL, or the
    // default repo standing in for a cwd yo doesn't know) is also where an
    // in-place session runs; one inferred from the cwd isn't.
    let mut repo_key: Option<String> = None;
    let mut repo_named = false;
    let mut profile_dir: Option<PathBuf> = None;
    let mut gh: Option<GhRef> = None;
    let mut url_repo: Option<String> = None;
    let mut unnamed = false;
    let mut msg = String::new();
    let mut name;
    match input {
        Input::Url(r) => {
            url_repo = reg.repo_for_url(sys, &r.url).map(|x| x.key.clone());
            let issue_repo = (r.kind == GhKind::Issue)
                .then(|| agent_repo.clone())
                .flatten();
            if url_repo.is_none() && repo_override.is_none() && issue_repo.is_none() && !here_flag {
                return Err(unknown_url_repo(ctx, &r.owner_repo));
            }
            repo_key = repo_override.clone().or(issue_repo).or(url_repo.clone());
            repo_named = repo_key.is_some();
            match r.kind {
                GhKind::Pr => {
                    name = format!("review_pr_{}", r.number);
                    msg = if agent.is_some() {
                        format!(
                            "Review the pull request {} as your instructions describe.",
                            r.url
                        )
                    } else {
                        format!("{} {}", reg.defaults.review_cmd, r.url)
                    };
                }
                GhKind::Issue => {
                    name = format!("issue_{}", r.number);
                    msg = if agent.is_some() {
                        format!(
                            "Take the issue {}: read it, its comments and any parent issue, then work it as your instructions describe.",
                            r.url
                        )
                    } else {
                        format!(
                            "take a look at the following issue, and work with me to address it: {} review the issue, its comments, any parent issue context you may need. then give me your take on it before doing any work. Issues may need to be broken down first, or we may only focus on a part of it.",
                            r.url
                        )
                    };
                }
            }
            gh = Some(r);
        }
        other => {
            match other {
                Input::Name(n) => name = n,
                _ => {
                    let names: Vec<String> = ctx
                        .ledger()
                        .all()
                        .into_iter()
                        .filter_map(|e| e.record.name)
                        .collect();
                    name = naming::unnamed(sys, &|n| names.iter().any(|t| t == n));
                    unnamed = true;
                }
            }
            if let Some(r) = &repo_override {
                repo_key = Some(r.clone());
                repo_named = true;
            } else if let Some(r) = agent_repo.clone().filter(|_| !here_flag) {
                repo_key = Some(r);
                repo_named = true;
            } else if let Some(p) = picked.as_deref().filter(|_| !here_flag) {
                match profile_target(ctx, p, &req.cwd)? {
                    Target::Repo(k, named) => {
                        repo_key = Some(k);
                        repo_named = named;
                    }
                    Target::Dir(d) => profile_dir = Some(d),
                }
            } else {
                repo_key = reg.detect_repo(sys, &req.cwd).map(|r| r.key.clone());
                if repo_key.is_none() && unnamed && !here_flag {
                    repo_key = reg.default_repo().map(|r| r.key.clone());
                    repo_named = repo_key.is_some();
                }
            }
        }
    }
    let repo: Option<&Repo> = repo_key.as_deref().and_then(|k| reg.repo(k));
    let harness = ctx.harness(&reg.harness(req.harness.as_deref(), repo, picked.as_deref()))?;
    check_flags(harness, req.model.is_some(), agent)?;
    let pr_review = gh.as_ref().is_some_and(|r| r.kind == GhKind::Pr);

    // Worktree or in place. A profile home that is no registered clone has
    // nothing to take a worktree from.
    let mode = if let Some(d) = &profile_dir {
        if req.mode == Some(Mode::Worktree) {
            return Err(Error::Refused(format!(
                "--worktree: {} isn't a registered clone, so there is nothing to take a worktree from",
                d.display()
            )));
        }
        Mode::Here
    } else {
        reg.mode(req.mode, repo, picked.as_deref())
    };

    if mode == Mode::Worktree {
        if let Some(r) = &gh {
            if pr_review {
                let Some(u) = &url_repo else {
                    return Err(unknown_url_repo(ctx, &r.owner_repo));
                };
                if repo_key.as_ref() != Some(u) {
                    return Err(Error::Refused(format!(
                        "--repo={} conflicts with PR URL repo ({u}): the worktree must come from the PR's repo",
                        repo_key.as_deref().unwrap_or("")
                    )));
                }
            } else if repo_key.is_none() {
                return Err(unknown_url_repo(ctx, &r.owner_repo));
            }
        }
        if repo.is_none() {
            return Err(Error::Refused(format!(
                "no repo given and couldn't infer one from {}\n        run inside a known clone, or: yo --repo=<repo> <INPUT>\n        (or --here, to work in this directory as it is)\n        known repos: {}",
                req.cwd.display(),
                repo_list()
            )));
        }
    }
    if let Some(r) = &gh {
        if r.kind == GhKind::Issue && repo_key.is_some() && repo_key != url_repo {
            msg.push_str(&format!(
                " NOTE: the issue lives in {}, but this session works in the {} repo — that's where the work is expected to land.",
                r.owner_repo,
                repo_key.as_deref().unwrap_or("")
            ));
        }
        if mode == Mode::Here && pr_review {
            msg.push_str(" NOTE: this session runs in place — the PR's head was NOT checked out, so the local tree may not be the code under review.");
        }
    }

    // The brief: the input's message, then EXTRA, then --from's report-back.
    let mut brief = msg;
    let mut add = |s: &str| {
        if !brief.is_empty() {
            brief.push(' ');
        }
        brief.push_str(s);
    };
    if let Some(e) = &extra {
        add(e);
    }
    if let Some(f) = &req.from {
        add(&harness.report_back(f));
    }
    let mut codename = naming::codename(sys);
    if let Some(d) = &agent_display {
        codename = format!("{d} · {codename}");
    }

    check_name(&name)?;

    let mut plan = Plan::default();
    let wt: PathBuf;
    if mode == Mode::Here {
        let dir = if let Some(d) = &profile_dir {
            if !d.is_dir() {
                return Err(Error::Refused(format!(
                    "profile '{}': no directory at {}\n        fix its dir in {}",
                    picked.as_deref().unwrap_or(""),
                    d.display(),
                    ctx.paths.config.display()
                )));
            }
            d.clone()
        } else if let (true, Some(r)) = (repo_named, repo) {
            if !r.path.is_dir() {
                return Err(no_clone(ctx, r));
            }
            r.path.clone()
        } else {
            req.cwd.clone()
        };
        // The physical path: it's what Claude keys its per-folder records on.
        wt = fs::canonicalize(&dir).map_err(|e| Error::io(&dir, e))?;
        plan.notes
            .push(Note::Info(format!("session in place: {}", wt.display())));
        if let Some(b) = git::current_branch(sys, &wt) {
            plan.notes.push(Note::Detail(format!("branch: {b}")));
        }
        plan.notes.push(Note::Detail(
            "as checked out — nothing fetched, pulled or created".into(),
        ));
        if let Some(r) = &gh {
            plan.notes.push(Note::Warn(if pr_review {
                format!(
                    "WARNING: PR #{} is NOT checked out — the review runs against the tree above",
                    r.number
                )
            } else {
                "WARNING: not starting from the latest base branch — pull first if that matters"
                    .into()
            }));
        }
        if let Some(a) = agent {
            check_agent_on_disk(ctx, harness, a, &wt)?;
        }
    } else {
        let r = repo.ok_or_else(|| Error::Refused("no repo".into()))?;
        let w = plan_worktree(
            ctx,
            &mut plan,
            &WorktreeAsk {
                repo: r,
                picked: picked.as_deref(),
                input: match (&gh, unnamed) {
                    (None, false) => Some(name.as_str()),
                    _ => None,
                },
                name: &name,
                pr: gh.as_ref().filter(|_| pr_review).map(|g| g.number),
                agent,
                harness,
            },
        )?;
        wt = w.path;
        name = w.name;
    }

    // The title: the repo's prefix, then the PR's or issue's title, the
    // codename for an unnamed session, else the name.
    let profile_name = reg
        .profile_of(repo, picked.as_deref())
        .map(|p| p.name.clone());
    let label = repo_key
        .clone()
        .or_else(|| profile_name.clone())
        .unwrap_or_else(|| {
            wt.file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
    let mut title = name.clone();
    if let Some(r) = &gh {
        if let Some(t) = github::title(sys, r) {
            let what = match r.kind {
                GhKind::Pr => "PR",
                GhKind::Issue => "Issue",
            };
            title = naming::truncate_title(&format!("{what}#{}: {t}", r.number));
        }
    } else if unnamed {
        title = codename.clone();
    }
    if let (Some(d), false) = (&agent_display, unnamed) {
        title = format!("{d} · {title}");
    }
    let prefix = match repo {
        Some(r) => reg.title_prefix(r),
        None => format!("[{label}] "),
    };
    title = format!("{prefix}{title}");
    if let Some(t) = &req.title {
        title = t.clone();
    }

    // A worktree yo just made from a trusted repo gets the clone's
    // per-folder state. A guest repo, or a profile with `trust: false`, is
    // code nobody here vetted: its trust dialog is left to appear. In place,
    // yo didn't make the directory, so trusting it stays the harness's
    // question.
    if let Some(p) = &profile_name {
        plan.notes.push(Note::Info(format!("profile: {p}")));
    }
    let trusted_clone = match (mode, repo) {
        (Mode::Worktree, Some(r)) if trusted(ctx, r) => Some(r.path.as_path()),
        _ => None,
    };
    if let (Mode::Worktree, Some(r), None) = (mode, repo, trusted_clone) {
        plan.notes.push(Note::Info(format!(
            "not pre-trusting the worktree ({} isn't a trusted repo) — {} will ask",
            r.key,
            harness.name()
        )));
    }
    let command = harness.command(reg, repo, picked.as_deref());
    let prepared = harness.prepare(
        ctx,
        &PrepareAsk {
            repo,
            picked: picked.as_deref(),
            label: &label,
            dir: &wt,
            mode,
            trusted_clone,
            command: &command,
        },
    )?;
    plan.notes.extend(prepared.notes);
    plan.actions.extend(prepared.actions);
    if let (Some(clone), Some(r)) = (trusted_clone, repo)
        && !r.mirror.is_empty()
    {
        plan.actions.push(Action::MirrorFiles {
            from: clone.to_path_buf(),
            to: wt.clone(),
            files: r.mirror.clone(),
        });
    }

    // yo picks the id where the harness lets it, so the ledger knows the
    // session before it starts: that's what lets resume-all bring back one
    // that crashed early. Otherwise the record gets its id once the
    // harness reports it; a session in place is still keyed by the id yo
    // made up, which is only a file name then.
    let made = sys.new_session_id();
    let id = (harness.ids() == IdPolicy::Picked).then(|| made.clone());
    let key = match mode {
        Mode::Here => Key::for_id(&made),
        Mode::Worktree => Key::for_worktree(&sys.home(), &wt),
    };
    let brief = (!brief.is_empty()).then_some(brief);
    let record = Record {
        worktree: wt.clone(),
        session_id: id.clone(),
        name: Some(name.clone()),
        title: Some(title.clone()),
        repo: repo_key.clone(),
        profile: profile_name,
        mode: Some(mode),
        agent: agent.map(str::to_string),
        guest: repo
            .filter(|r| reg.guest.as_deref() == Some(r.key.as_str()))
            .and_then(|r| reg.github_for(sys, r)),
        codename: Some(codename.clone()),
        brief: brief.clone(),
        from: req.from.clone(),
        by_id: mode == Mode::Here,
        harness: Some(harness.name().to_string()),
        runner: Some(RunnerKind::Iterm),
        created_at: Some(sys.unix_now()),
        ..Record::default()
    };
    plan.actions.push(Action::PutRecord(Box::new(Entry {
        key: key.clone(),
        record,
    })));

    let spec = SessionSpec {
        command,
        dir: wt.clone(),
        name: name.clone(),
        id,
        model: req.model.clone(),
        persona: agent.map(str::to_string),
        codename: codename.clone(),
        first_message: brief.clone(),
        settings: prepared.settings,
        args: prepared.args,
        remote_control: None,
    };
    let mut argv = harness.tui_command(&spec);
    argv.env.insert(0, ("YO_SESSION_KEY".into(), key.0.clone()));

    let group = reg
        .window_group(repo, picked.as_deref())
        .replace("{repo}", &label);
    plan.actions.push(Action::Launch(LaunchSpec {
        dir: wt,
        title,
        argv,
        window: req.window,
        group,
        record: Some(key),
    }));
    Ok(plan)
}

struct Parsed {
    repo_override: Option<String>,
    picked: Option<String>,
    input: Input,
    extra: Option<String>,
}

/// The flags and positional arguments, checked against the registry.
fn parse(ctx: &Ctx, req: &CreateRequest) -> Result<Parsed> {
    let reg = &ctx.registry;
    let repo_list = || {
        reg.repos()
            .iter()
            .map(|r| r.key.as_str())
            .collect::<Vec<_>>()
            .join(" | ")
    };
    let mut repo_override = req.repo.clone();
    if let Some(r) = &repo_override
        && reg.repo(r).is_none()
    {
        return Err(Error::Refused(format!(
            "--repo must be one of: {} (got: '{r}')",
            repo_list()
        )));
    }
    let mut picked = req.profile.clone();
    if let Some(p) = &picked
        && reg.profile(p).is_none()
    {
        let names: Vec<&str> = reg.profiles().iter().map(|p| p.name.as_str()).collect();
        return Err(Error::Refused(format!(
            "--profile must be one of: {} (got: '{p}')",
            names.join(" | ")
        )));
    }

    // `yo <repo> ...` and `yo <profile> ...`: a repo of that name wins.
    let mut args: &[String] = &req.args;
    if let Some(first) = args.first() {
        if reg.repo(first).is_some() {
            repo_override = Some(first.clone());
            args = &args[1..];
        } else if picked.is_none() && repo_override.is_none() && reg.profile(first).is_some() {
            picked = Some(first.clone());
            args = &args[1..];
        }
    }
    if args.len() > 2 {
        return Err(Error::Refused(
            "usage: yo [flags] [REPO|PROFILE] [INPUT] [EXTRA]".into(),
        ));
    }
    let input = match args.first().map(String::as_str) {
        None | Some("" | "-") => Input::Unnamed,
        Some(s) => match github::parse_ref(s) {
            Some(r) => Input::Url(r),
            None => Input::Name(s.to_string()),
        },
    };
    let extra = args.get(1).filter(|s| !s.is_empty()).cloned();

    Ok(Parsed {
        repo_override,
        picked,
        input,
        extra,
    })
}

/// The `owner/repo` of a PR or issue URL whose repo isn't registered and
/// has to be placed (see [`place`]) before the session can be planned:
/// nothing else (`--repo`, `--here`, an agent's repo for an issue) says
/// where the session goes.
pub fn unplaced(ctx: &Ctx, req: &CreateRequest) -> Result<Option<String>> {
    let reg = &ctx.registry;
    let Parsed {
        repo_override,
        input: Input::Url(r),
        ..
    } = parse(ctx, req)?
    else {
        return Ok(None);
    };
    let agent_repo = req
        .agent
        .as_deref()
        .and_then(|a| reg.agent(a))
        .map(|a| a.repo.clone());
    let pointed = repo_override.is_some()
        || req.mode == Some(Mode::Here)
        || (r.kind == GhKind::Issue && agent_repo.is_some());
    let known = reg.repo_for_url(&*ctx.sys, &r.url).is_some();
    Ok((!known && !pointed).then_some(r.owner_repo))
}

/// Whether yo pre-trusts `repo`'s worktrees: not a guest, and not when its
/// own profile says `trust: false`.
pub(crate) fn trusted(ctx: &Ctx, repo: &Repo) -> bool {
    if ctx.registry.guest.as_deref() == Some(repo.key.as_str()) {
        return false;
    }
    repo.profile
        .as_deref()
        .and_then(|p| ctx.registry.profile(p))
        .is_none_or(|p| p.trust)
}

/// A session name: letters, digits, `.`, `_`, `/`, `-`. It becomes a path
/// and a branch, so not one that climbs out or reads as a flag.
fn check_name(name: &str) -> Result<()> {
    let chars_ok = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'));
    if !chars_ok || name.is_empty() {
        return Err(Error::Refused(format!(
            "name may only contain letters, digits, . _ / - (got: '{name}')"
        )));
    }
    let bad_part = name
        .split('/')
        .any(|p| p.is_empty() || p.chars().all(|c| c == '.') || p.starts_with('-'));
    if bad_part {
        return Err(Error::Refused(format!(
            "name can't have empty or dot-only parts, or start one with '-' (got: '{name}')"
        )));
    }
    Ok(())
}

enum Target {
    /// A repo, and whether it counts as named.
    Repo(String, bool),
    /// The profile's home, which is no registered clone.
    Dir(PathBuf),
}

/// Where a session goes when a profile was picked and no repo named: its
/// home directory (a clone, or a directory to run in place), else the
/// cwd's repo when it's one of the profile's, else the profile's first.
fn profile_target(ctx: &Ctx, picked: &str, cwd: &Path) -> Result<Target> {
    let reg = &ctx.registry;
    if let Some(dir) = reg.profile(picked).and_then(|p| p.dir.clone()) {
        return Ok(match reg.repo_at(&dir) {
            Some(r) => Target::Repo(r.key.clone(), true),
            None => Target::Dir(dir),
        });
    }
    if let Some(r) = reg.detect_repo(&*ctx.sys, cwd)
        && r.profile.as_deref() == Some(picked)
    {
        return Ok(Target::Repo(r.key.clone(), false));
    }
    match reg.repos_of_profile(picked).next() {
        Some(r) => Ok(Target::Repo(r.key.clone(), true)),
        None => Err(Error::Refused(format!(
            "profile '{picked}' has no dir and no repos\n        give it one in {}, or name a repo: yo --profile={picked} --repo=<repo>",
            ctx.paths.config.display()
        ))),
    }
}

struct WorktreeAsk<'a> {
    repo: &'a Repo,
    picked: Option<&'a str>,
    /// A plain INPUT, which may name an existing branch.
    input: Option<&'a str>,
    name: &'a str,
    /// The PR under review.
    pr: Option<u64>,
    agent: Option<&'a str>,
    harness: &'a dyn Harness,
}

struct Worktree {
    path: PathBuf,
    /// The name, as the branch made it.
    name: String,
}

/// The worktree and its branch: an existing branch named verbatim, else
/// `<prefix>/<name>` (reused locally, tracked from origin, or new from the
/// base branch or the PR's head).
fn plan_worktree(ctx: &Ctx, plan: &mut Plan, w: &WorktreeAsk<'_>) -> Result<Worktree> {
    let reg = &ctx.registry;
    let sys = &*ctx.sys;
    let clone = &w.repo.path;
    if !clone.join(".git").exists() {
        return Err(no_clone(ctx, w.repo));
    }
    let base = reg.base_ref(sys, w.repo);
    let upstream = git::remote_for(sys, clone, reg.github_for(sys, w.repo).as_deref());
    if let Some(a) = w.agent {
        let rev = format!("{upstream}/{base}");
        if !has_agent_at(ctx, w.harness, clone, &rev, a) {
            return Err(no_agent(w.harness, a, &format!("{rev}:"), clone));
        }
    }

    let prefix = reg.branch_prefix(Some(w.repo), w.picked);
    let explicit = w
        .input
        .filter(|i| git::has_local_branch(sys, clone, i) || git::has_origin_branch(sys, clone, i));
    let (branch, name) = if let Some(b) = explicit {
        let n = b
            .strip_prefix(&format!("{prefix}/"))
            .unwrap_or(b)
            .replace('/', "-");
        (b.to_string(), n)
    } else {
        let mut n = w.name.to_string();
        if n.contains('/') {
            let flat = n.replace('/', "-");
            plan.notes.push(Note::Info(format!(
                "note: '{n}' doesn't match any existing branch; treating it as a session name (flattened: {flat})"
            )));
            n = flat;
        }
        let b = if prefix.is_empty() {
            n.clone()
        } else {
            format!("{prefix}/{n}")
        };
        (b, n)
    };
    check_name(&name)?;
    let mut root = clone.as_os_str().to_owned();
    root.push("-worktrees");
    let path = PathBuf::from(root).join(&name);
    if path.exists() {
        return Err(Error::Refused(format!(
            "worktree path already exists: {}",
            path.display()
        )));
    }

    let reuse = git::has_local_branch(sys, clone, &branch);
    let remote_reuse = !reuse && w.pr.is_none() && git::has_origin_branch(sys, clone, &branch);
    let fetch = |remote: &str, refspec: String, what: String| Action::Fetch {
        repo: clone.clone(),
        remote: remote.to_string(),
        refspec,
        what,
    };
    let spec = if reuse {
        if let Some(m) = git::branch_merge(sys, clone, &branch) {
            plan.notes.push(Note::Detail(format!(
                "note: {branch} tracks {m} — 'git pull' in the worktree to refresh"
            )));
        }
        BranchSpec::Existing(branch.clone())
    } else if let Some(n) = w.pr {
        plan.actions.push(fetch(
            &upstream,
            format!("pull/{n}/head"),
            format!("PR #{n} head (from {upstream})"),
        ));
        BranchSpec::New {
            branch: branch.clone(),
            start: "FETCH_HEAD".into(),
        }
    } else if remote_reuse {
        plan.actions.push(fetch(
            "origin",
            branch.clone(),
            format!("origin/{branch} (existing remote branch)"),
        ));
        BranchSpec::Track {
            branch: branch.clone(),
            start: format!("origin/{branch}"),
        }
    } else {
        plan.actions
            .push(fetch(&upstream, base.clone(), format!("{upstream}/{base}")));
        BranchSpec::New {
            branch: branch.clone(),
            start: format!("{upstream}/{base}"),
        }
    };
    plan.actions.push(Action::AddWorktree {
        repo: clone.clone(),
        path: path.clone(),
        branch: spec,
    });
    if let (Some(n), false) = (w.pr, reuse) {
        // `git pull` in the worktree then pulls new PR commits.
        plan.actions.push(Action::SetUpstream {
            repo: clone.clone(),
            branch,
            remote: upstream,
            merge: format!("refs/pull/{n}/head"),
        });
    }
    Ok(Worktree { path, name })
}

fn no_clone(ctx: &Ctx, r: &Repo) -> Error {
    Error::Refused(format!(
        "no clone at: {}\n        clone '{}' there, or fix its path in {}",
        r.path.display(),
        r.key,
        ctx.paths.config.display()
    ))
}

fn unknown_url_repo(ctx: &Ctx, owner_repo: &str) -> Error {
    let known: Vec<&str> = ctx
        .registry
        .repos()
        .iter()
        .map(|r| r.key.as_str())
        .collect();
    Error::Refused(format!(
        "unrecognized repo in URL: {owner_repo}\n        known repos: {}\n        register it: yo config add-repo <clone|url>",
        known.join(" | ")
    ))
}

/// `--model` and `--agent`, for a harness that may have neither: an
/// error naming it, never silently dropped.
pub(crate) fn check_flags(h: &dyn Harness, model: bool, agent: Option<&str>) -> Result<()> {
    let caps = h.capabilities();
    if model && !caps.models {
        return Err(Error::Refused(format!(
            "--model: the {} harness can't pick a model",
            h.name()
        )));
    }
    if agent.is_some() && (!caps.personas || h.persona_dir().is_none()) {
        return Err(Error::Refused(format!(
            "--agent: the {} harness has no agents",
            h.name()
        )));
    }
    Ok(())
}

pub(crate) fn no_agent(h: &dyn Harness, name: &str, at: &str, dir: &Path) -> Error {
    let d = h.persona_dir().unwrap_or_default();
    Error::Refused(format!(
        "no agent named '{name}' in {at}{d} of {}, or in ~/{d}\n        yo starts only agents that are merged (or installed in ~/{d})",
        dir.display()
    ))
}

/// Whether agent `name` is defined in the harness's persona directory at
/// `rev` of `clone` (merged), or in the one under `HOME`.
pub(crate) fn has_agent_at(
    ctx: &Ctx,
    h: &dyn Harness,
    clone: &Path,
    rev: &str,
    name: &str,
) -> bool {
    let Some(d) = h.persona_dir() else {
        return false;
    };
    git::has_agent_at(&*ctx.sys, clone, rev, d, name) || home_has_agent(ctx, d, name)
}

/// For a session in place: the agent is in the directory's persona
/// directory, or in the one under `HOME`.
fn check_agent_on_disk(ctx: &Ctx, h: &dyn Harness, name: &str, dir: &Path) -> Result<()> {
    let found = h
        .persona_dir()
        .is_some_and(|d| dir_has_agent(&dir.join(d), name) || home_has_agent(ctx, d, name));
    if found {
        Ok(())
    } else {
        Err(no_agent(h, name, "", dir))
    }
}

fn home_has_agent(ctx: &Ctx, persona_dir: &str, name: &str) -> bool {
    dir_has_agent(&ctx.sys.home().join(persona_dir), name)
}

/// Whether any file under `dir` defines agent `name` (a `name:` line).
fn dir_has_agent(dir: &Path, name: &str) -> bool {
    let Ok(rd) = fs::read_dir(dir) else {
        return false;
    };
    rd.filter_map(|e| e.ok()).any(|e| {
        let p = e.path();
        if p.is_dir() {
            dir_has_agent(&p, name)
        } else {
            fs::read_to_string(&p).is_ok_and(|t| defines_agent(&t, name))
        }
    })
}

/// `name: NAME` on a line of its own, quoted or not: what
/// [`git::agent_pattern`] matches.
fn defines_agent(text: &str, name: &str) -> bool {
    text.lines().any(|l| {
        let Some(v) = l.strip_prefix("name:") else {
            return false;
        };
        let v = v.trim();
        let v = v.strip_prefix(['"', '\'']).unwrap_or(v);
        let v = v.strip_suffix(['"', '\'']).unwrap_or(v);
        v == name
    })
}

#[cfg(test)]
mod tests;
