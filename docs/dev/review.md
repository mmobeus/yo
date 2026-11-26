# How a review is done

The rules every reviewer follows, whichever checklist it applies
([architecture](architecture.md), [security](security.md),
[tests](tests.md)) and whichever harness runs it. In Claude Code these are
the `review-*` skills in `.claude/skills/`, and `review-pr` runs all three.

## What's reviewed

- **A diff and the code it applies to.** `review-pr` gives each reviewer a
  diff file, a checkout of the commit under review, and a checkout of the
  base branch.
- **Run by itself**, a reviewer reviews the current branch's change:
  `git diff $(git merge-base origin/main HEAD)`, which includes uncommitted
  changes, plus untracked files from `git status`.
- **Rules from the base, code from the change.** The checklists, the review
  skills and `AGENTS.md` are read from the base checkout. A PR doesn't get
  reviewed by its own edited reviewer. Everything else, including
  `docs/design/core.md` as changed, is read from the checkout under review.
  Run by itself, both are the current directory.

## What's not read

The PR description, commit messages, the issue, PR comments, or anything
else the author wrote about the change. The reviewer judges what the change
does, not what it says it does. If the session running a reviewer already
knows the author's explanation, it doesn't pass it on.

## Scope

- **Your checklist only.** The other reviewers cover their areas, and
  general correctness is the built-in `/code-review`'s job.
- **Not what the gates already enforce.** `cargo xtask ci` checks the lints,
  the forbidden dependencies and the banned calls. Report those only when
  the diff gets around a gate or loosens it.
- **Read around the change** when you need to. A diff hides what's next to
  it.

## Findings

- **Concrete:** the file and line (in the new version), what's wrong, the
  rule it breaks (a checklist item or a design section), and what would fix
  it.
- **Blocking or not:** as the checklist puts it. If you're unsure whether
  something is a problem, make it non-blocking and say what you're unsure
  of.
- **No padding:** no praise, no summary of the diff, no findings invented to
  fill a section.

## Format

```
## <area>

### Blocking
- `path:line`: what's wrong. Rule: <checklist item or design section>. Fix: <what would fix it>.

### Non-blocking
- `path:line`: ...
```

Write `none` under an empty heading. A reviewer may add sections after
these when its skill asks for them (security's inventory).
