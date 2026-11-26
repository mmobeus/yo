---
name: review-pr
description: Run yo's three review skills (architecture, security, tests) on a pull request, each as a separate reviewer that sees only the diff and the design. Posts one PR comment with the findings and sets the yo-review commit status on the commit reviewed.
argument-hint: "[PR number]"
disable-model-invocation: true
---

# review-pr

Run by hand, on a PR. The rules every reviewer follows are in
`docs/dev/review.md`.

## 1. The PR and the commit

```sh
gh pr view $ARGUMENTS --json number,headRefOid,baseRefName,url
```

With no argument, that's the current branch's PR. Note the number `N`, the
head commit `SHA` and the base branch `BASE`. The review is of `SHA`, even if
the PR moves on while it runs.

## 2. Checkouts and the diff

```sh
git fetch origin "$BASE" "pull/$N/head"
dir=$(mktemp -d)
git worktree add --detach "$dir/base" "origin/$BASE"
git worktree add --detach "$dir/src" "$SHA"
git diff "$(git merge-base "origin/$BASE" "$SHA")" "$SHA" > "$dir/diff.patch"
```

## 3. Three reviewers

Start three `yo-reviewer` agents, in parallel, in one message, one for each
of `review-architecture`, `review-security` and `review-tests`. Each prompt
is only this, with the paths filled in:

> Follow `<dir>/base/.claude/skills/review-<area>/SKILL.md`.
> The diff under review: `<dir>/diff.patch`.
> The base checkout (the skills, `docs/dev/`, `AGENTS.md`): `<dir>/base`.
> The checkout under review (everything else): `<dir>/src`.

Don't add the PR's title, description, commits, issue, or your own summary
of the change. Reviewers that see the author's explanation tend to agree
with it.

## 4. One comment

Put the three outputs together without editing their findings. If two
reviewers report the same problem, keep both. Count the blocking and
non-blocking findings. Then post:

```markdown
## yo review: `<short SHA>`

Commit `<SHA>`, against `<BASE>`. Each reviewer saw only the diff, the code
and the design, not this PR's description.

**<n> blocking, <m> non-blocking.**

<architecture output>

<security output, with its inventory>

<tests output>

---

The `yo-review` status on `<short SHA>` records that this review ran. It
doesn't mean the findings were dealt with: that's the maintainer's check
before merging.
```

```sh
gh pr comment "$N" --body-file "$dir/comment.md"
```

It prints the comment's URL.

## 5. The status

```sh
gh api "repos/{owner}/{repo}/statuses/$SHA" \
  -f state=success -f context=yo-review \
  -f description="review ran: <n> blocking, <m> non-blocking" \
  -f target_url="<comment URL>"
```

The state is always `success`. The status records that review ran. The
blocking findings are in the comment, for the maintainer.

## 6. Clean up and report

```sh
git worktree remove --force "$dir/base"
git worktree remove --force "$dir/src"
rm -rf "$dir"
```

If `gh pr view "$N" --json headRefOid` now shows a different commit, say so:
the new head has no `yo-review` status and needs its own review. Tell the
user the counts, list the blocking findings, and give the comment's URL.
