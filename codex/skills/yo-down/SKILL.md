---
name: yo-down
description: Tear down the current yo work session from inside it — remove its git worktree (keeping the branch), forget it in yo's session ledger, and close its iTerm2 tab, which ends this Codex session. Use when the user says "yo down", "tear this session down", "we're done here, clean up the worktree", or invokes $yo-down.
---

# yo-down

`yo down` removes the worktree this session runs in and closes its iTerm2 tab.
Closing the tab **ends this conversation**, so the last thing you say has to come
before the command, not after it.

## Steps

1. **Report first, change nothing.** From the worktree root (the dry run only
   reads, so the sandbox is fine):

   ```sh
   cd "$(git rev-parse --show-toplevel 2>/dev/null || pwd)" && yo down --dry-run
   ```

   If it says there is no yo worktree here, this is an in-place session: no
   files are touched, and teardown only forgets the session in yo's ledger and
   closes the tab. Skip to step 3. Don't hunt for another directory to tear
   down.

2. **Show the user what the dry run found.** Lead with anything that would be lost:
   - **Uncommitted or untracked changes** are deleted for good. List them and
     ask whether to commit, stash, or discard them. Never decide this yourself.
   - **Commits not on the base branch** survive on the branch, which is kept.
     Say whether they're pushed (`git status -sb`). Unpushed commits are safe
     but only exist on this machine.

3. **Get an explicit go-ahead in this conversation.** The user invoking
   `$yo-down` isn't a go-ahead on its own: they haven't seen the dry run yet.
   If the tree is dirty, they must explicitly agree to **discard** the changes
   before you pass `--discard`. If they want to keep them, commit or stash as
   they ask, then run step 1 again.

4. **Say goodbye, then run it.** Your last message should note the session is
   ending and, for a worktree, that the worktree is being removed and the
   branch is kept. Then, as the final command, outside the sandbox: it
   removes the worktree, writes yo's ledger under `~/.local/state/yo` and
   closes the tab through iTerm2, all of which the sandbox blocks. Ask for
   escalated permissions on the first try; a sandboxed attempt fails partway.

   ```sh
   cd "$(git rev-parse --show-toplevel 2>/dev/null || pwd)" && yo down --yes
   ```

   Add `--discard` only after step 3's explicit agreement. The tab closes, so
   this command usually returns no result. That's expected, not a failure.

## When the tab doesn't close

If `yo down` prints `not running inside an iTerm2 session`, it did the rest but
there was no tab to close. Tell the user the session is still open and they can
end it with `/quit`.
