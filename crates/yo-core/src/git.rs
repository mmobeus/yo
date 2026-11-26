//! The git CLI, through [`Sys::cmd`]. Only what's ported so far.

use std::path::{Path, PathBuf};

use crate::sys::{CommandSpec, Sys};

/// `git -C dir args...`, its trimmed stdout when it succeeds.
pub fn git(sys: &dyn Sys, dir: &Path, args: &[&str]) -> Option<String> {
    let spec = CommandSpec::new("git")
        .arg("-C")
        .arg(dir.to_string_lossy())
        .args(args.iter().copied());
    let out = sys.cmd().run(&spec).ok()?;
    out.success().then(|| out.text())
}

/// Whether `git -C dir args...` succeeds.
fn git_ok(sys: &dyn Sys, dir: &Path, args: &[&str]) -> bool {
    git(sys, dir, args).is_some()
}

/// The clone's remotes, in git's order.
pub fn remotes(sys: &dyn Sys, dir: &Path) -> Vec<String> {
    git(sys, dir, &["remote"])
        .map(|s| s.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

pub fn remote_url(sys: &dyn Sys, dir: &Path, remote: &str) -> Option<String> {
    git(sys, dir, &["remote", "get-url", remote]).filter(|u| !u.is_empty())
}

/// The clone's default branch, for repos configured without a `base`:
/// `origin/HEAD`, else the first of main, master and dev on origin, else
/// main.
pub fn default_branch(sys: &dyn Sys, clone: &Path) -> String {
    if let Some(r) = git(
        sys,
        clone,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    ) {
        return r.strip_prefix("origin/").unwrap_or(&r).to_string();
    }
    for b in ["main", "master", "dev"] {
        let r = format!("refs/remotes/origin/{b}");
        if git_ok(sys, clone, &["show-ref", "--verify", "--quiet", &r]) {
            return b.to_string();
        }
    }
    "main".to_string()
}

/// The primary clone behind `dir`: in a worktree, `--git-common-dir` is the
/// main clone's `.git`, given as an absolute path; in the clone itself it's
/// relative, and the clone is the top level.
pub fn primary_clone(sys: &dyn Sys, dir: &Path) -> Option<PathBuf> {
    let top = git(sys, dir, &["rev-parse", "--show-toplevel"])?;
    let common = git(sys, dir, &["rev-parse", "--git-common-dir"])?;
    let common = Path::new(&common);
    if common.is_absolute() {
        Some(common.parent().unwrap_or(common).to_path_buf())
    } else {
        Some(PathBuf::from(top))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{FakeSys, ok};

    #[test]
    fn default_branch_order() {
        let sys = FakeSys::new("/h");
        assert_eq!(default_branch(&sys, Path::new("/r")), "main");
        sys.answer(
            "git",
            &[
                "-C",
                "/r",
                "show-ref",
                "--verify",
                "--quiet",
                "refs/remotes/origin/master",
            ],
            ok(""),
        );
        assert_eq!(default_branch(&sys, Path::new("/r")), "master");
        sys.answer_ok(
            "git",
            &[
                "-C",
                "/r",
                "symbolic-ref",
                "--short",
                "refs/remotes/origin/HEAD",
            ],
            "origin/trunk\n",
        );
        assert_eq!(default_branch(&sys, Path::new("/r")), "trunk");
    }

    #[test]
    fn primary_clone_from_a_worktree_and_the_clone() {
        let sys = FakeSys::new("/h");
        let top = ["-C", "/c", "rev-parse", "--show-toplevel"];
        let common = ["-C", "/c", "rev-parse", "--git-common-dir"];
        sys.answer_ok("git", &top, "/c\n");
        sys.answer_ok("git", &common, ".git\n");
        assert_eq!(
            primary_clone(&sys, Path::new("/c")),
            Some(PathBuf::from("/c"))
        );
        sys.answer_ok("git", &top, "/c-worktrees/x\n");
        sys.answer_ok("git", &common, "/c/.git\n");
        assert_eq!(
            primary_clone(&sys, Path::new("/c")),
            Some(PathBuf::from("/c"))
        );
        assert_eq!(primary_clone(&sys, Path::new("/elsewhere")), None);
    }
}
