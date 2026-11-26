# applescript-message-in-script-text

- base: `6df999b893a1f355a1912e50d8dbfeba61646dc3`
- expect: flag, with a blocking finding citing `scripts/yo`

The message yo types into a session is pasted into the AppleScript's text (an unquoted heredoc, `set msg to "$message"`) instead of passed as an argument. A `"` in the message injects AppleScript. The first planted problem, from #11's "done when".
