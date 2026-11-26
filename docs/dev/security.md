# Security checklist

For reviewing a diff. yo runs commands, removes worktrees, types into
terminals and (later) listens on a socket, so its mistakes can cost work or
open a door. "Security tests" in [the design](../design/core.md) lists the
tests that back these up.

**Blocking** means the PR shouldn't merge as is. **Non-blocking** means worth
fixing, now or in a follow-up.

## Blocking

- **Destructive operations.** `sweep` and `down` never remove a dirty
  worktree, the primary clone, the current directory, an agent's home, or a
  worktree a live session still holds. Branches with unpushed or unmerged
  work aren't deleted without being asked. A new destructive action goes
  through a plan, honors `--dry-run`, and asks before acting unless told not
  to.
- **Shell quoting.** Anything passed through a shell is quoted by the one
  single-quoting helper. Prefer an argv list over a shell string. Never
  `sh -c` with text interpolated in.
- **AppleScript.** Messages, titles and paths reach `osascript` as
  arguments (`on run argv`), never pasted into the script's text.
- **Paths.** Session names, branch names, issue and PR URLs, and config
  values can't make a path that escapes the worktree root or the state
  directory (`..`, absolute paths, symlinks).
- **New outside commands.** Each new program yo runs is named in the PR. Its
  arguments come from validated values, and its output is parsed, not
  trusted.
- **New inputs.** A new flag, config key, hook payload field, ledger field,
  event, API request or ACP message is validated where it enters. The hook
  never fails or hangs on a malformed payload, because it runs at every
  `claude` start and exit.
- **The event log.** Newlines and control characters in any text can't
  break the JSON lines format.
- **The daemon's boundary.** It listens on a Unix socket only, unless TCP is
  turned on. The socket and token file are `0600`. Every request needs the
  token. Over TCP it never binds all interfaces, and checks `Origin` and
  `Host`.
- **The app.** It's granted only the Tauri commands the UI uses.
- **The relay.** A relayed message whose recipient or text changed on the
  way is a failed delivery. A relayed message is never shown or reported as
  the user's own voice.
- **Secrets.** No tokens, keys or personal data in the repo, logs, the
  ledger, the event log, test fixtures or PR text. Local secrets live in
  the gitignored `.env`, or in the Keychain for the app.
- **CI.** Workflows have least `permissions`, actions are pinned by commit
  hash, and nothing uses `pull_request_target` or exposes secrets to PR
  code.

## Non-blocking (unless it's serious)

- **New dependencies.** Each one is justified in the PR and added to
  `bans.allow`. It uses minimal features and comes from crates.io. A
  dependency with a build script, or one that is unmaintained, gets a
  closer look.
- **Error messages** don't leak tokens or full environment dumps.
- **File permissions** for anything yo writes that only the user should
  read (tokens, state).
