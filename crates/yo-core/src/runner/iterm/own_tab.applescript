-- The tab of the iTerm2 session with unique id (argument 1), as
-- "<window id>/<tab id>", or nothing.
on run argv
  set want to item 1 of argv
  tell application "iTerm2"
    repeat with ww in (get windows)
      repeat with tt in (get tabs of ww)
        repeat with ss in (get sessions of tt)
          if (unique id of ss) is want then
            try
              tell ss
                return ((variable named "tab.window.id") as text) & "/" & ((variable named "tab.id") as text)
              end tell
            end try
            return ""
          end if
        end repeat
      end repeat
    end repeat
  end tell
  return ""
end run
