---
name: review-security
description: Review a yo diff for security (docs/dev/security.md) - destructive operations, shell and AppleScript quoting, paths, the daemon's boundary, the app's permissions, the relay, secrets - and list its new outside commands, inputs and dependencies. Use when asked to review yo changes for security, or as part of review-pr.
argument-hint: "[diff file] [checkout dir]"
---

# review-security

Read `docs/dev/review.md` first: it says what to review,
what not to read, and the output format. Then:

1. Read `docs/dev/security.md`: the checklist. Read "Security tests" in
   `docs/design/core.md`, and the parts of it about whatever the diff
   touches (the daemon, the relay, the app, the ledger hook).
2. **Follow untrusted text.** For every value that comes from outside
   (arguments, config, hook payloads, ledger and event files, branch and
   session names, PR and issue titles, URLs, ACP messages, HTTP requests),
   trace where it goes. Quoting and path findings usually come from this.
   A value interpolated into a shell string, an AppleScript's text or a path
   without the helper or a check is blocking.
3. **Destructive operations:** for anything that removes, overwrites or
   kills, check what stops it hitting the wrong target. Read the code
   around it, not just the diff.
4. Go through the rest of the checklist. What the code already did the
   same way before the diff isn't a finding, unless the diff makes it
   worse.
5. **Inventory.** Whatever you find, list what the diff adds:
   - outside commands (programs run, and with what)
   - new inputs (where outside data now enters)
   - new dependencies (crates, actions, tools)

   Write "none" for an empty one. Add this after the findings, under
   `### Inventory`.

The output's area is `security`.
