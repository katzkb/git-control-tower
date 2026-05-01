# F3 — Worktree post-create hooks
# Show .gct.toml hooks running automatically: .env copy + command echo.
#
# Regenerate via: make demos-f3

Output docs/images/f3-hooks.gif

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 720
Set Theme "Dracula"
Set TypingSpeed 50ms
Set Padding 20

Hide
Type 'eval "$(bash scripts/demo/setup.sh f3)"'
Enter
Sleep 800ms
Show

# Show the project's .gct.toml so the hooks are in the viewer's mind
Type "cat .gct.toml"
Sleep 300ms
Enter
Sleep 2200ms

Type "clear"
Sleep 200ms
Enter
Sleep 300ms

Type "gct"
Sleep 300ms
Enter
Sleep 1800ms

# Move to feature/payments (main is selected by default)
Down
Sleep 600ms

# Create worktree — hooks fire automatically (.env copy, then command)
Type "w"
Sleep 2800ms

# Confirm cd into the new worktree
Type "y"
Sleep 1800ms

# Prove .env was actually copied by the post_create hook
Type "ls -A"
Sleep 300ms
Enter
Sleep 2000ms
