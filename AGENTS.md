# Working on yo

For any coding agent (or person) changing this repo. yo is built mostly by
coding agents, so every change clears the same gates, whoever wrote it.

## Read first

- [`docs/design/core.md`](docs/design/core.md): the plan for moving yo from
  the bash script to a Rust core. Its status table says what's built. Read
  its principles and "How it's built" before changing anything in `crates/`.
- [`docs/dev/`](docs/dev/): how to run the gates
  ([`env.md`](docs/dev/env.md)), and the checklists every change is reviewed
  against: [architecture](docs/dev/architecture.md),
  [security](docs/dev/security.md), [tests](docs/dev/tests.md).

## Layout

```
scripts/yo        the yo that runs today (bash); the reference for the port
crates/           the Rust workspace (see "Crates" in the design)
xtask/            the gates: cargo xtask <gate>
claude/           Claude Code files yo installs for its users (skills, settings)
.claude/          Claude Code files for working on this repo
docs/             the user guide, the reference, the design, docs/dev/
```

Until phase 1 lands, `scripts/yo` is what users run, and the guide and
reference describe it. A change to its behavior updates them.

## Before opening a PR

1. `cargo xtask ci` passes. It runs exactly what CI runs.
2. If you added or changed a dependency: every new crate is in `bans.allow`
   in `deny.toml`, and `cargo xtask advisories` passes.
3. If you changed `scripts/yo`: no gate covers it yet. Try it in a sandbox
   (`YO_DRY_LAUNCH=1`, with `HOME`, `XDG_STATE_HOME` and `YO_CONFIG` pointed
   at a throwaway directory; see the README's "Making changes").
4. Go through the three checklists in `docs/dev/` for your diff.
5. Fill in the PR template.

## After opening it

Run the review from the main checkout, on an up-to-date, clean `main`, not
from the PR's branch: `/review-pr <number>` in Claude Code. (Run from the
PR's branch, the PR's own edited reviewer would review it.) With any other
harness, follow
`.claude/skills/review-pr/SKILL.md` by hand: it's plain markdown, and the
reviewers' rules are in [`docs/dev/review.md`](docs/dev/review.md). It posts
one comment with blocking and non-blocking findings, and sets the
`yo-review` status on the commit it reviewed. A new push needs a new review.
Deal with the blocking findings, or say in the PR why one is wrong. Fixes
that only address the review's findings don't need a new review: the
maintainer may merge past the missing `yo-review` with an admin bypass.
That's the maintainer's call, not the author's. The
built-in `/code-review` covers general correctness, and is worth running too.

If a bug turns up after merging that review should have caught, file an
issue labeled `review-miss`. `tune-review-skills` (run at the end of a
phase, or after a miss) turns misses into cases and better skills; see
[`docs/dev/review-tuning.md`](docs/dev/review-tuning.md).

The reviewers don't read the PR description. A reason they need to see
(why an output changed, why a check is the way it is) goes in the code or
beside the entry, not only in the PR.

## The design doc stays true

A PR that changes what the design says, or lands part of a phase, updates
`docs/design/core.md` in the same PR:

- **The status table:** a phase's row moves from "not started" to
  "in progress (#issue): what's done" to "done (#issue, PR #n)".
- **Anything that turned out different** from the doc is corrected where the
  doc says it, not left for later.

If the change and the doc disagree and you think the doc is wrong, say so in
the PR rather than quietly building something else.

## Gates are only loosened by the maintainer

Don't weaken a check to get a PR green. That includes:

- removing or relaxing a lint, or adding an `#[allow]` for a denied one
  (outside `sys.rs` for `yo-core`'s banned calls)
- `deny.toml`: a license, a duplicate skipped, a source, or a bans rule
- `crates/yo-core/clippy.toml`, or the forbidden dependencies in `xtask`
- deleting, skipping or `#[ignore]`-ing a test, or relaxing its assertions,
  or an `accepted.diff` entry
  that hides a change in effects
- the review skills' example cases (`cases/`)

If a change really needs one of these, do it in its own commit and list it
under "Gates loosened" in the PR. The maintainer approves it explicitly in
that PR, or it doesn't merge. Adding a new crate to `bans.allow` isn't
loosening, but say why the crate is needed.

## Conventions

- **Work in a worktree**, not in the checkout installed into `~`.
- **Branches** are short and descriptive (`issue_11`, `ledger-codename`),
  with no user prefix.
- **Commits:** `area: what changed`, lowercase, for example
  `yo: --model for create and resume`, `docs: ...`, `build: ...`, or a crate
  (`core: ...`). The body says why.
- **No personal or employer information** in commits, branches, PRs, docs or
  code. yo may become public.
- **No AI attribution lines:** no `Co-Authored-By` trailers for coding
  agents, and no "Generated with ..." lines in PRs, issues or comments.
- **PRs** link their issue (`Closes #n` or `Part of #n`) and say what was
  checked by hand.
