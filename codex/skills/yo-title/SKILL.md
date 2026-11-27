---
name: yo-title
description: Set this session's iTerm2 tab title, which is also what iTerm's Session Status lists and what `yo resume` restores. Use when the user invokes $yo-title, asks to rename / retitle / label this session or tab, or asks you to title it after what you're working on.
---

# yo-title

1. **Pick the title.** If the user gave one, use it exactly. Otherwise write one
   from the work in this session: what's being done, not the branch name. Keep
   it to about 40 characters, e.g. `sweep: fork-aware base ref` or
   `PR 1260 review: partition flush`. Include an issue or PR number if there is one.

2. **Set it** (quote it as one argument), outside the sandbox: it talks to
   iTerm2 and writes yo's ledger under `~/.local/state/yo`, which the sandbox
   blocks. Ask for escalated permissions on the first try rather than running
   it sandboxed and retrying, and suggest the `prefix_rule` `["yo", "title"]`,
   so the user can approve it once for good.

   ```sh
   yo title '<title>'
   ```

   It finds this session's own tab by its iTerm2 session ID, whichever tab has
   focus. It renames every pane in the tab and records the title so a resume
   brings it back.

3. **Report it in one line.** If the output says the session isn't in the
   ledger, add that `$yo-adopt` will make the title survive a restart.

If it fails with `not inside an iTerm2 session`, the session isn't running in
iTerm. Say so and stop.
