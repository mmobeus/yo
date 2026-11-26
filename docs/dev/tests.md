# Tests checklist

For reviewing a diff's tests. "Testing the port" and "How it's built" in
[the design](../design/core.md) say what each layer is for.

**Blocking** means the PR shouldn't merge as is. **Non-blocking** means worth
fixing, now or in a follow-up.

## Blocking

- **Changed behavior has a test**, at the right layer. "Behavior" is what a
  user, a command or a later step relies on: a rule (precedence, a lookup's
  order, what's refused), an effect, an error a user sees. Not every branch
  or helper needs its own test; a missing one is blocking only when a real
  rule could break unnoticed.
  - **Logic** (resolution, precedence, sweep's rules, naming, status, plans):
    unit tests in `yo-core`, with a fake `Sys`.
  - **What a command does** (its effects): unit tests in `yo-core` on the
    plan it builds, and on running that plan against the fake `Sys` under a
    temporary `HOME`. A new command or flag gets tests for its error paths
    too.
  - **What a command prints:** `yo-cli` tests that render its result
    against the fake `Sys` and assert on the text. The core never prints,
    so its tests can't check output.
  - **A security rule:** the matching security test (quoting, paths,
    destructive operations, the daemon, the hook, the event log).
- **A bug fix adds a test** that fails without the fix.
- **Changed output is documented.** A port that makes a command behave or
  print differently from `scripts/yo` adds it to the list in "Testing the
  port" in the design, in the same diff, where reviewers can see that it's
  deliberate. (The reference describes `scripts/yo` until the switch.)
- **Dry runs:** a command that changes things has a test that runs it with
  `--dry-run`: it runs no command that changes anything, and its temporary
  `HOME` is unchanged afterwards.
- **Deterministic and sandboxed.** Tests use the fake `Sys` and temporary
  directories. No network, no real `HOME`, no iTerm, no real harness. They
  don't sleep to wait for something. They pass on Linux.
- **No weakened tests.** Nothing is deleted, skipped, `#[ignore]`d or has
  its assertions relaxed to get green without the maintainer's approval in the PR (see `AGENTS.md`).

## Non-blocking (unless it's serious)

- **Tests check behavior, not structure.** Assert what happens, not how
  it's implemented.
- **Don't test the tests.** A test whose job is to check that another test,
  lint or gate is set up right is noise. When a new gate goes in, check by
  hand once that it catches what it should, and say so in the PR, for the
  maintainer (the reviewers don't see it, and don't check it). Not
  catching everything is fine. A wall of guards against obvious, reviewable
  mistakes isn't.
- **Fakes stay honest.** A fake behaves like the real thing in the ways the
  test depends on. A harness fake with no optional capabilities runs the
  command tests too.
- **Fixtures are small** and say what they're for.
