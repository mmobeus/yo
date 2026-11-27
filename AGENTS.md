# Working on yo

For any coding agent (or person) changing this repo. yo is built mostly by
coding agents, so every change clears the same gates, whoever wrote it.

## Read first

- [`docs/design/core.md`](docs/design/core.md): the plan for moving yo from
  a bash script to a Rust core and beyond. Its status table says what's
  built. Read its principles and "How it's built" before changing anything in `crates/`.
- [`docs/dev/`](docs/dev/): how to run the gates
  ([`env.md`](docs/dev/env.md)), how a release is cut
  ([`release.md`](docs/dev/release.md)), and the checklists every change is reviewed
  against: [architecture](docs/dev/architecture.md),
  [security](docs/dev/security.md), [tests](docs/dev/tests.md).

## Layout

```
crates/           the Rust workspace (see "Crates" in the design); yo-cli is
                  the `yo` binary
xtask/            the gates: cargo xtask <gate>
claude/           Claude Code files yo installs for its users (skills, settings)
codex/            Codex files yo installs for its users (the same skills, for Codex)
.claude/          Claude Code files for working on this repo
docs/             the user guide, the reference, the design, docs/dev/
```

`yo setup` installs `claude/` and `codex/` from copies built into the binary (`yo setup
--link <checkout>` links them instead). The guide and the reference describe
every command: a change to what a command does or prints updates them in the
same PR.

## Before opening a PR

1. `cargo xtask ci` passes. It runs exactly what CI runs.
2. If you added or changed a dependency: every new crate is in `bans.allow`
   in `deny.toml`, and `cargo xtask advisories` passes.
3. If you changed what a command does: try it by path
   (`target/debug/yo`) in a sandbox (`YO_DRY_LAUNCH=1`, with `HOME`,
   `XDG_STATE_HOME` and `YO_CONFIG` pointed at a throwaway directory; see
   the README's "Making changes"), and say in the PR what you tried. If you
   changed `yo setup`, try it the same way, with `--no-iterm` (iTerm2's
   preferences aren't under `HOME`).
4. Go through the three checklists in `docs/dev/` for your diff.
5. Fill in the PR template.

## After opening it

Start the review yourself, in a new session: don't leave it for the
maintainer. It runs from the main checkout, on an up-to-date, clean `main`,
not from the PR's branch (run from the PR's branch, the PR's own edited
reviewer would review it). Check the main checkout is on `main`, clean and
current with `origin/main`, then, from it:

```sh
yo --here --from=<your session's name> --title='Review PR #<number>' - '/review-pr <number>'
```

That opens a Claude Code session in place in the main checkout with
`/review-pr <number>` as its first prompt. `--from` has it message your
session (with `SendMessage`) when the review is posted or it gets stuck,
and it stays open for the rounds below.
Your session's name is its Claude name: the name `yo sessions` lists for it
(for a session yo started, its worktree's name). Tell the maintainer the
review is running. If the main checkout isn't clean and on `main`, say so
rather than changing it. With any other harness, follow
`.claude/skills/review-pr/SKILL.md` by hand, from the main checkout: it's
plain markdown, and the reviewers' rules are in
[`docs/dev/review.md`](docs/dev/review.md). It posts
one comment with blocking and non-blocking findings, and sets the
`yo-review` status on the commit it reviewed, so a new push has no
`yo-review` until it's reviewed again.

The findings are a second opinion, not a ruling: the reviewers are guessing
as much as you are. Fix the blocking ones you agree with; for one you
don't, say in a line on the PR why not. Every non-blocking finding gets one
of three outcomes, said on the PR:

- **Fixed now**, when it's right (you agree, having looked), it's about
  what this PR added or changed (not an older problem nearby), and it's
  small (a few lines, no new design decision, nothing that needs tests or
  docs of its own). A finding that the PR left a doc, comment or test
  untrue always counts as small.
- **An issue**, labeled `review-followup`, when it's right and worth doing
  but isn't about this PR's change, or isn't small. Link it on the PR.
- **Declined**, in a line, when it isn't right or isn't worth doing.

Make the fixes in the same push as the blocking ones, so one round checks
them. The
built-in `/code-review` covers general correctness, and is worth running too.

### Review rounds

A review that's accepted ends the cycle; one that isn't starts another
round. At most three rounds, the first review included:

1. **Ask the reviewer to confirm.** Once your fixes are pushed (and the
   rest answered on the PR), message the reviewer's session (the
   `from-name` its message to you came with) with `SendMessage`: the PR,
   the commit to check, one line per blocking finding saying fixed or why
   not, and each non-blocking one's outcome. If the first review had no blocking findings and you pushed
   nothing since, it's accepted already: skip to step 2.
2. **Accepted:** the reviewer reviews what changed, posts its comment
   saying it's accepted, sets `yo-review` on that commit, and tells you.
   Merge the PR, the accepted commit and nothing after it
   (`gh pr merge <number> --merge --match-head-commit <sha>`): anything you
   push later needs another round, so a non-blocking finding left after
   acceptance becomes an issue rather than a push. If the
   merge is refused (CI hasn't passed, or a check is missing), say so to
   the maintainer; don't bypass it.
3. **Not accepted:** its comment lists what's still blocking. Weigh each
   again: fix it, or say on the PR why not (filing an issue where it's
   worth doing later), then go back to step 1.
4. **After the third round without acceptance, stop.** Don't merge, and
   don't ask for a fourth. Tell the maintainer what's still open on the
   PR: from here it's theirs to resolve.
5. **Then tear down the review sessions you started**, once the PR is
   merged or step 4 stops the rounds: `yo down <review session>` for each
   (`yo sessions --from=<your session's name>` lists them, and
   `yo down --children` tears down the ones that are done). After that,
   tear yourself down (`/yo-down`), or say you're ready to be.

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
- deleting, skipping or `#[ignore]`-ing a test, or relaxing its assertions
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
