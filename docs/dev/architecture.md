# Architecture checklist

For reviewing a diff against [the design](../design/core.md). The mechanical
rules (forbidden dependencies, no printing in `yo-core`, banned outside
calls) are already enforced by `cargo xtask ci`. This list covers what those
can't check.

**Blocking** means the PR shouldn't merge as is. **Non-blocking** means worth
fixing, now or in a follow-up.

## Blocking

- **The right crate.** Site logic goes in `yo-core`. `yo-cli` parses,
  renders and prompts, and holds no decisions. `yo-proto` holds only types
  and the API trait. `yo-app` forwards to the daemon and holds no yo logic.
  `yo-local` and `yo-client` implement the same trait and behave the same.
  See "Crates" in the design.
- **`yo-core` stays synchronous.** No async, no background threads, no
  long-lived state. Anything that needs a long-running process belongs in
  the daemon.
- **Every outside effect goes through `Sys`.** That covers commands, the
  clock, ids, randomness, the environment, the process table and the home
  directory. Files are touched under paths that `Sys` gives. Watch for ways
  around the lint: a new helper crate that does it, `libc`, or a path built
  from something other than `Sys`.
- **The core returns; callers report.** `yo-core` returns errors and
  results. It doesn't print, exit, or prompt.
- **Commands that change things build a plan first.** The plan is what
  `--dry-run` shows, and a dry run executes none of it. See "Commands as
  plans".
- **Claude-specific is fine, Claude-assumed isn't.** Nothing outside
  `harness/claude/` assumes Claude's files, flags, settings layer, memory,
  hooks or Remote Control. A harness that lacks a capability shows up as
  "unknown" or "not available", never as an error. The same goes for
  runners and iTerm.
- **Effects match `scripts/yo`** during the port: the ledger, config, git,
  files written and commands launched. Only output may differ, and only
  deliberately. See "Parity test plan".
- **The design doc is still true.** If the diff does something the doc
  doesn't describe, or does differently, the doc is updated in the same PR.
  The status table reflects what landed.
- **No gate is loosened** without the maintainer's approval in the PR. See
  `AGENTS.md`.

## Non-blocking (unless it's serious)

- **Two of everything general.** A new trait meant to be general has a
  second implementation in this phase, or a planned one.
- **Build on what harnesses ship.** Before building something, check
  whether the harness already provides it (status, background sessions,
  messaging), and use it through the adapter.
- **Config and ledger formats** stay as they are unless there's a reason. A
  new field is documented in the reference.
- **git, gh and osascript** are run as CLIs, not replaced with libraries.
- **Types are the contract.** Shared shapes (config, records, events, the
  API) are serde types in one place, not ad-hoc JSON.
- **Simple ownership:** owned data, `Arc` where something is shared, and no
  long-lived borrowed references in APIs.
- **Heavy dependencies** stay out of crates that don't need them, so builds
  stay fast.
