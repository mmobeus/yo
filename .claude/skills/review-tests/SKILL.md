---
name: review-tests
description: Review a yo diff's tests (docs/dev/tests.md) - tests at the right layer for each change, dry runs, determinism, nothing weakened. Use when asked to review yo changes' tests, or as part of review-pr.
argument-hint: "[diff file] [checkout dir]"
---

# review-tests

Read `docs/dev/review.md` first: it says what to review,
what not to read, and the output format. Then:

1. Read `docs/dev/tests.md`: the checklist. Read "Testing the port" in
   `docs/design/core.md`.
2. **List the behavior changes** in the diff: new or changed logic, commands,
   flags, outputs, effects, error paths and bug fixes. Then find the test for
   each one, at the layer the checklist gives. A behavior change with no
   test is blocking. Docs-only or gate-only changes need none.
3. **Weakened tests:** anything deleted, skipped, `#[ignore]`d, or with its
   assertions relaxed is blocking, with the rule "needs the maintainer's
   approval".
4. Go through the rest of the checklist. A test that only checks another
   test's, lint's or gate's setup is a non-blocking finding ("don't test the
   tests").

The output's area is `tests`.
