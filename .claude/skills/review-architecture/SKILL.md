---
name: review-architecture
description: Review a yo diff against the design's principles and architecture rules (docs/dev/architecture.md), and check the design doc and its status table are still true. Use when asked to review yo changes for architecture, or as part of review-pr.
argument-hint: "[diff file] [checkout dir]"
---

# review-architecture

Read `docs/dev/review.md` first: it says what to review,
what not to read, and the output format. Then:

1. Read `docs/dev/architecture.md`: the checklist. Read the parts of
   `docs/design/core.md` the diff touches: always Principles, Crates and
   "How it's built"; "The core's API", Harnesses and Runners when the code
   touches them.
2. For each changed file, ask which crate it's in, and whether the change
   belongs there.
3. Go through the checklist's blocking items, then its non-blocking ones.
4. **Is the design doc still true?** Compare what the diff does with what the
   doc says. If the code behaves differently from what the doc describes,
   that's blocking unless the diff also updates the doc. Signatures and
   sketches that differ in their details, while the behavior agrees, aren't
   a finding. If the
   diff lands part of a phase, the status table's row says so.
5. **Gates loosened:** a change to the gates' own setup (lint levels,
   `deny.toml`, `crates/yo-core/clippy.toml`, the forbidden dependencies in
   `xtask`, CI) goes under "For the maintainer", not "Blocking". Say exactly
   what was loosened. Approving it is the maintainer's call, not yours.
   Code that gets around a gate is different: an `#[allow]` that lets
   `yo-core` make a banned call outside `sys.rs`, say, breaks the rule the
   lint enforces, and is blocking.

The output's area is `architecture`.
