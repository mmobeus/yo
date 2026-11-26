-- The title yo gave the iTerm2 session with unique id (argument 1), its
-- user.yoTitle, or nothing. From bash's cmd_adopt.
on run argv
  set want to item 1 of argv
  tell application "iTerm2"
    repeat with ww in windows
      repeat with tt in tabs of ww
        repeat with ss in sessions of tt
          if (unique id of ss) is want then
            try
              tell ss to set v to variable named "user.yoTitle"
              if v is not missing value then return v
            end try
            return ""
          end if
        end repeat
      end repeat
    end repeat
  end tell
  return ""
end run
