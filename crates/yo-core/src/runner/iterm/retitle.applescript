-- Claude Code's workgroup renames the main session ("Chat") a moment after
-- claude starts; Session Status rows show the session name, so put yo's title
-- back once it has. Run detached. From bash's reassert_session_title.
on run argv
  set want to item 1 of argv
  -- Poll rather than sleep-once: the rename lands whenever claude first prints
  -- enough for the trigger to fire, and a workgroup can be re-entered later.
  repeat 12 times
    delay 2
    tell application "iTerm2"
      repeat with ww in windows
        repeat with tt in tabs of ww
          repeat with ss in sessions of tt
            set v to ""
            try
              tell ss
                set v to variable named "user.yoTitle"
              end tell
            end try
            if v is want and (name of ss) is not want then set name of ss to want
          end repeat
        end repeat
      end repeat
    end tell
  end repeat
end run
