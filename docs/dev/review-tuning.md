# Review tuning log

`tune-review-skills` (in `.claude/skills/`) adds a row here each time it
runs, and starts from the last row the next time. It runs at the end of a
phase, or after a miss.

A **miss** is a bug found after merging that review should have caught. File
it as an issue with the `review-miss` label, and link the PR that brought it
in. A finding that was wrong is noted in a reply on that PR's review comment.

| date | PR | what changed |
| --- | --- | --- |
| 2026-11-26 | #16 | the first cases: one to flag and one to pass for each skill |
