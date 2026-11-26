-- Brings forward the iTerm2 session on a terminal (argument 1) and types a
-- message (argument 2) into it, if any. Prints "found" or "notfound". From
-- bash's focus_running_session.
on run argv
  set want to item 1 of argv
  set msg to item 2 of argv
  tell application "iTerm2"
    repeat with ww in windows
      repeat with tt in tabs of ww
        repeat with ss in sessions of tt
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
  end tell
  return "notfound"
end run
