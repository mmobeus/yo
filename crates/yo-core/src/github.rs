//! GitHub names out of URLs and checkout paths, and the gh CLI for PR and
//! issue titles.

use crate::sys::{CommandSpec, Sys};

/// A PR or an issue, as a URL names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GhRef {
    pub kind: GhKind,
    pub owner_repo: String,
    pub number: u64,
    /// The canonical URL: anything after the number (`/files`, an anchor) dropped.
    pub url: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GhKind {
    Pr,
    Issue,
}

/// A `github.com/<owner>/<repo>/(pull|issues)/<n>` URL, anywhere in `s`.
pub fn parse_ref(s: &str) -> Option<GhRef> {
    let (_, rest) = s.split_once("github.com/")?;
    let mut parts = rest.splitn(4, '/');
    let (owner, repo, seg) = (parts.next()?, parts.next()?, parts.next()?);
    let tail = parts.next()?;
    let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
    let number: u64 = digits.parse().ok()?;
    let kind = match seg {
        "pull" => GhKind::Pr,
        "issues" => GhKind::Issue,
        _ => return None,
    };
    if !(name_ok(owner) && name_ok(repo)) {
        return None;
    }
    Some(GhRef {
        kind,
        owner_repo: format!("{owner}/{repo}"),
        number,
        url: format!("https://github.com/{owner}/{repo}/{seg}/{number}"),
    })
}

/// The PR's or issue's title, from gh. `None` when gh isn't there or fails.
pub fn title(sys: &dyn Sys, r: &GhRef) -> Option<String> {
    let what = match r.kind {
        GhKind::Pr => "pr",
        GhKind::Issue => "issue",
    };
    let spec = CommandSpec::new("gh").args([
        what,
        "view",
        &r.number.to_string(),
        "--repo",
        &r.owner_repo,
        "--json",
        "title",
        "--jq",
        ".title",
    ]);
    let out = sys.cmd().run(&spec).ok()?;
    out.success().then(|| out.text()).filter(|t| !t.is_empty())
}

/// `owner/repo` out of any GitHub remote or web URL (its first two path
/// segments, without `.git`). Exact, so `foo/bar` can't match `foo/bar-baz`.
pub fn owner_repo_from_url(url: &str) -> Option<String> {
    let u = url.strip_suffix(".git").unwrap_or(url);
    let rest = if let Some((_, r)) = u.split_once("github.com:") {
        r
    } else {
        u.split_once("github.com/")?.1
    };
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    let (owner, rest) = rest.split_once('/')?;
    let repo = rest.split('/').next().unwrap_or("");
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    (name_ok(owner) && name_ok(repo)).then(|| format!("{owner}/{repo}"))
}

/// A GitHub owner or repo name: not empty, not only dots, not starting with
/// `-` (it would read as a flag), and no characters GitHub doesn't allow.
fn name_ok(n: &str) -> bool {
    !n.is_empty()
        && !n.starts_with('-')
        && !n.chars().all(|c| c == '.')
        && n.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// `owner/repo` out of a conventional `.../github.com/<owner>/<repo>`
/// checkout path. Only when the path really has `github.com/` in it, so it
/// can't invent an owner for a clone kept somewhere else.
pub fn owner_repo_from_path(path: &str) -> Option<String> {
    let (_, rest) = path.split_once("/github.com/")?;
    let rest = rest.trim_end_matches('/');
    let (owner, rest) = rest.split_once('/')?;
    let repo = rest.split('/').next().unwrap_or("");
    (name_ok(owner) && name_ok(repo)).then(|| format!("{owner}/{repo}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        for (url, want) in [
            ("git@github.com:acme/api.git", Some("acme/api")),
            ("https://github.com/acme/api", Some("acme/api")),
            ("https://github.com/acme/api/pull/3/files", Some("acme/api")),
            ("ssh://git@github.com/acme/api.git", Some("acme/api")),
            ("https://github.com/acme", None),
            ("https://gitlab.com/acme/api", None),
            ("https://github.com//api", None),
            ("https://github.com/../api", None),
            ("https://github.com/acme/..", None),
            ("https://github.com/-x/api", None),
            ("https://github.com/acme/a%20b", None),
        ] {
            assert_eq!(owner_repo_from_url(url).as_deref(), want, "{url}");
        }
    }

    #[test]
    fn refs() {
        let r = parse_ref("https://github.com/acme/api/pull/12/files#x").unwrap();
        assert_eq!(r.kind, GhKind::Pr);
        assert_eq!(r.owner_repo, "acme/api");
        assert_eq!(r.number, 12);
        assert_eq!(r.url, "https://github.com/acme/api/pull/12");
        let r = parse_ref("github.com/acme/api/issues/7").unwrap();
        assert_eq!((r.kind, r.number), (GhKind::Issue, 7));
        assert_eq!(parse_ref("https://github.com/acme/api/tree/main"), None);
        assert_eq!(parse_ref("https://github.com/acme/api/pull/x"), None);
        assert_eq!(parse_ref("https://github.com/../api/pull/1"), None);
        assert_eq!(parse_ref("issue_7"), None);
    }

    #[test]
    fn paths() {
        assert_eq!(
            owner_repo_from_path("/h/dev/src/github.com/acme/api").as_deref(),
            Some("acme/api")
        );
        assert_eq!(
            owner_repo_from_path("/h/dev/src/github.com/acme/api/").as_deref(),
            Some("acme/api")
        );
        assert_eq!(owner_repo_from_path("/h/src/acme/api"), None);
        assert_eq!(owner_repo_from_path("/h/github.com/acme"), None);
    }
}
