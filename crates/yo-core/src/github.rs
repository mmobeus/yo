//! GitHub names out of URLs and checkout paths. Pure; the gh CLI comes with
//! the commands that use it.

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
