-- Retitles the iTerm2 tab holding the session with unique id (argument 1),
-- every pane in it, to argument 2. Prints "ok" or "missing". From bash's
-- cmd_title.
on run argv
  set want to item 1 of argv
  set newTitle to item 2 of argv
  tell application "iTerm2"
    repeat with ww in (get windows)
      repeat with tt in (get tabs of ww)
        set hit to false
        repeat with ss in (get sessions of tt)
          if (unique id of ss) is want then set hit to true
        end repeat
        if hit then
          -- every pane in the tab: Session Status lists them all
          repeat with ss in (get sessions of tt)
            tell ss
              set variable named "user.yoTitle" to newTitle
              set name to newTitle
            end tell
          end repeat
          return "ok"
        end if
      end repeat
    end repeat
  end tell
  return "missing"
end run
