---
name: yo-adopt
description: Record the current Codex session in yo's session ledger so `yo resume-all` brings it back after a reboot or crash. Use when the user invokes $yo-adopt, or asks to "register / track / adopt this session with yo", or to make it survive a restart — typically for sessions yo didn't start.
---

# yo-adopt

Run this, exactly as written, outside the sandbox: it writes yo's ledger
under `~/.local/state/yo` and asks iTerm2 for this tab's title, which the
sandbox blocks. Ask for escalated permissions on the first try rather than
running it sandboxed and retrying.

```sh
yo adopt
```

`yo` reads this session's ID from `CODEX_THREAD_ID` and the launch directory
from Codex's own records (its rollout). It also reuses the iTerm2 tab title
when yo set one. Pass nothing else unless the user gave a title, in which case
use `yo adopt --title='<their title>'`.

Then tell the user, in one or two lines, what was recorded: the title, session
ID and directory from the output. Add that `yo resume-all` after a restart will
reopen it. Running it again is harmless.

If it fails with `no session id`, or `yo` isn't found, say so and stop. Don't
work around it by writing ledger files yourself.
