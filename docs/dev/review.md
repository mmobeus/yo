# How a review is done

The rules every reviewer follows, whichever checklist it applies
([architecture](architecture.md), [security](security.md),
[tests](tests.md)) and whichever harness runs it. In Claude Code these are
the `review-*` skills in `.claude/skills/`, and `review-pr` runs all three.

## What a review is

A second opinion, not a ruling. A reviewer sees less than the author does
(no PR description, no issue, no conversation) and is guessing as much as
the author is. The author weighs each finding: fixes the ones that are
right, says in a line why not for the ones that aren't, and leaves most
non-blocking ones for later. The maintainer decides what has to change
before merging, and whether another review is worth running.

So a review aims for the few findings that matter, not for everything that
could be said.

## What's reviewed

- **A diff and the code it applies to.** `review-pr` gives each reviewer a
  diff file, a checkout of the commit under review, and a checkout of the
  base branch.
- **Run by itself**, a reviewer reviews the current branch's change:
  `git diff $(git merge-base origin/main HEAD)`, which includes uncommitted
  changes, plus untracked files from `git status`.
- **A re-review is of what changed since the last one.** When the PR
  already has a review comment, the diff is from the commit that review
  names to the new head, and the reviewer also checks whether that
  review's blocking findings were dealt with (fixed, or answered with a
  reason). It doesn't start over on the whole PR.
- **Rules from the base, code from the change.** `review-pr` runs from a
  clean checkout of the base branch, since the harness loads the skill and
  the reviewer agent from there. The checklists, the review skills and
  `AGENTS.md` are read from the base checkout. A PR doesn't get reviewed by
  its own edited reviewer. Everything else, including
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

- **The bar.** Report something only if it would plausibly cause a bug, a
  security problem, or a broken rule from your checklist, in what this diff
  does now. Not a finding:
  - doubts about code nothing uses yet, or about what a later step might
    do with it
  - wording, naming, or a design sketch whose details differ from the code
    while the behavior agrees (the doc says its signatures are "the shape,
    not the final code")
  - hardening beyond what `scripts/yo` does today, when the port keeps
    bash's behavior; only a diff that makes things worse than bash is a
    finding
  - a test for every branch; see the tests checklist for what needs one
- **Blocking means wrong now:** a bug, a hole, or a rule clearly broken.
  "Could be better" or "might matter later" is non-blocking at most. If
  you're unsure something is a problem at all, leave it out.
- **Few.** At most three non-blocking findings, the ones that matter most.
  Leave the rest out; don't list them in brief.
- **Concrete:** the file and line (in the new version, as a path relative
  to the checkout, not the temporary directory it's in), what's wrong, the
  rule it breaks (a checklist item or a design section), and what would fix
  it.
- **No padding:** no praise, no summary of the diff, no findings invented to
  fill a section.

## Format

```
## <area>

### Blocking
- `path:line`: what's wrong. Rule: <checklist item or design section>. Fix: <what would fix it>.

### Non-blocking
- `path:line`: ...

### For the maintainer
- what the diff loosens (a lint, a `deny.toml` rule, a ban, a test, a case)
```

Write `none` under an empty heading. "For the maintainer" lists each change
to a gate's own setup that loosens it, once, for the maintainer to approve
in the PR. It isn't blocking and isn't counted: the reviewers can't see the
approval, so making it blocking would leave it open forever. A reviewer may
add sections after these when its skill asks for them (security's
inventory).
