# Hero demo — pick a review-requested PR, drop it into a local worktree,
# then mime out the next-tool step (claude / idea .).
#
# Regenerate via: make demos-hero

Output docs/images/hero.gif

Set Shell "bash"
Set FontSize 14
Set Width 1200
Set Height 720
Set Theme "Dracula"
Set TypingSpeed 50ms
Set Padding 20

Hide
Type 'eval "$(bash scripts/demo/setup.sh hero)"'
Enter
Sleep 800ms
Show

Type "gct"
Sleep 300ms
Enter
Sleep 1800ms

# Switch to Review filter — sidebar fills with review-requested PRs
Type "3"
Sleep 2000ms

# First entry is "feature/api-redesign #138" — the streaming refactor PR.
# Create worktree directly via the main-view shortcut.
Type "w"
Sleep 2800ms

# gct auto-prompts "Move to Worktree?" after creation. Confirm with 'y'.
# This exits gct; the gct shell function (from `gct shell-init`) then cd's
# into the new worktree path.
Type "y"
Sleep 1800ms

# Now in a shell inside the worktree. Mime "open it in your tool of choice".
Type "claude"
Sleep 800ms
Backspace 6
Sleep 400ms
Type "idea ."
Sleep 800ms
Backspace 6
Sleep 1200ms
