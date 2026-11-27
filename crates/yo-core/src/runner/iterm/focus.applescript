-- Brings forward the iTerm2 session on a terminal (argument 1) and types a
-- message (argument 2) into it, if any. Prints "found" or "notfound". From
-- bash's focus_running_session.
--
-- If no session shown is on that terminal, the tab "<window id>/<tab id>"
-- (argument 3, if any) is brought forward instead, and "tab" printed: a
-- workgroup tab showing CLI lists only its CLI session. Nothing is typed
-- then, since it would go to the shell.
--
-- Every loop here and in the other scripts walks `(get windows)` and so
-- on: references by id. A plain `windows` refers to them by position, and
-- `select ww` brings that window to the front and renumbers them, so the
-- next line would reach into another window ("Can't get item 3 of every
-- tab of item 3 of every window"): the window came forward, but not its
-- tab, and the message was never typed.
on run argv
  set want to item 1 of argv
  set msg to item 2 of argv
  set wantTab to item 3 of argv
  tell application "iTerm2"
    repeat with ww in (get windows)
      repeat with tt in (get tabs of ww)
        repeat with ss in (get sessions of tt)
          if (tty of ss) is want then
            select ww
            tell tt to select
            tell ss to select
            activate
            if msg is not "" then
              -- Claude reads a long burst of input as a paste, and a newline
              -- inside a paste is a line break, not Enter: type the message,
              -- pause, then press Enter on its own.
              tell ss to write text msg newline no
              delay 0.5
              tell ss to write text (character id 13) newline no
            end if
            return "found"
          end if
        end repeat
      end repeat
    end repeat
    if wantTab is "" then return "notfound"
    repeat with ww in (get windows)
      repeat with tt in (get tabs of ww)
        repeat with ss in (get sessions of tt)
          set seen to ""
          try
            tell ss
              set seen to ((variable named "tab.window.id") as text) & "/" & ((variable named "tab.id") as text)
            end tell
          end try
          if seen is wantTab then
            select ww
            tell tt to select
            activate
            return "tab"
          end if
        end repeat
      end repeat
    end repeat
  end tell
  return "notfound"
end run
