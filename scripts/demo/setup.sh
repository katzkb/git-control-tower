#!/usr/bin/env bash
# Create a deterministic fixture git repo for a VHS demo recording.
# Stdout: eval-able shell commands that cd into the repo and export the
# env vars gh-stub needs. The tape Sources this output via:
#   eval "$(bash scripts/demo/setup.sh <scene>)"

set -euo pipefail

SCENE="${1:?usage: setup.sh <scene>}"
REPO_ROOT="${GCT_REPO_ROOT:?GCT_REPO_ROOT must be set (project root)}"
FIXTURES="$REPO_ROOT/scripts/demo/fixtures"
GH_STUB="$REPO_ROOT/scripts/demo/gh-stub"

# Frozen author + date so commits hash the same every run
AUTHOR_NAME="Demo User"
AUTHOR_EMAIL="demo@example.com"
GIT_DATE="2024-06-15T10:00:00Z"
export GIT_AUTHOR_NAME="$AUTHOR_NAME"   GIT_COMMITTER_NAME="$AUTHOR_NAME"
export GIT_AUTHOR_EMAIL="$AUTHOR_EMAIL" GIT_COMMITTER_EMAIL="$AUTHOR_EMAIL"
export GIT_AUTHOR_DATE="$GIT_DATE"      GIT_COMMITTER_DATE="$GIT_DATE"

# Reap fixture dirs from previous runs before creating a fresh one. The
# current run's $TMP can't safely remove itself with an EXIT trap because
# the recorded shell's cwd may sit inside it (after `cd into worktree`),
# and VHS's session-end isn't guaranteed to fire EXIT traps anyway.
# Cleaning at the *start* of each run is robust, leaves at most one stale
# dir between runs, and never races the recording.
find "$(dirname "$(mktemp -u)")" -maxdepth 1 -type d -name 'gct-demo-*' -mmin +1 \
  -exec rm -rf {} + 2>/dev/null || true

TMP="$(mktemp -d -t "gct-demo-${SCENE}.XXXXXX")"
REPO="$TMP/repo"
mkdir -p "$REPO" "$TMP/bin" "$TMP/wt" "$TMP/empty-template"

# Drop gh-stub on PATH as `gh`
cp "$GH_STUB" "$TMP/bin/gh"
chmod +x "$TMP/bin/gh"

cd "$REPO"
# Empty template dir avoids the user's global init.templatedir, which can
# carry personal pre-commit/prepare-commit-msg hooks that would run against
# demo commits. core.hooksPath is also pinned to a non-existent path as a belt.
git init --initial-branch=main --template="$TMP/empty-template" --quiet
git config user.name  "$AUTHOR_NAME"
git config user.email "$AUTHOR_EMAIL"
git config commit.gpgsign false
git config core.hooksPath "$TMP/empty-template"
git remote add origin "https://github.com/demo-user/demo-project.git"

echo "# demo-project" > README.md
git add README.md
git commit --quiet -m "Initial commit"

mkdir -p src
cat > src/main.rs <<'RS'
fn main() {
    println!("Hello, demo!");
}
RS
git add src/
git commit --quiet -m "feat: scaffold project"

# Helper: create a branch off main with one extra commit, then return to main
make_branch() {
  local name="$1" msg="$2"
  git checkout -b "$name" --quiet
  local fname
  fname="src/$(echo "$name" | tr '/' '_').txt"
  echo "$msg" > "$fname"
  git add "$fname"
  git commit --quiet -m "$msg"
  git checkout main --quiet
}

case "$SCENE" in
  hero)
    make_branch "feature/auth-flow"     "feat(auth): scaffold login form"
    make_branch "feature/api-redesign"  "refactor: extract repository layer"
    make_branch "fix/wt-progress"       "fix: smooth worktree progress flicker"
    make_branch "chore/deps"            "chore: bump tokio to 1.43"
    ;;
  f1)
    make_branch "feature/done-search"   "feat: branch search"
    make_branch "feature/done-filter"   "feat: filter modes"
    make_branch "fix/done-flicker"      "fix: sidebar flicker"
    make_branch "fix/done-encoding"     "fix: utf-8 encoding"
    make_branch "chore/done-clippy"     "chore: clippy lints"
    make_branch "feature/wip-export"    "wip: export to JSON"
    make_branch "fix/wip-cache"         "wip: cache invalidation"
    # Merge the "done" set so they show up under git branch --merged
    for b in feature/done-search feature/done-filter fix/done-flicker fix/done-encoding chore/done-clippy; do
      git merge --no-ff --quiet "$b" -m "Merge $b"
    done
    ;;
  f2)
    make_branch "feature/auth-flow"     "feat(auth): login form"
    make_branch "feature/auth-tokens"   "feat(auth): tokens"
    make_branch "feature/billing"       "feat: billing"
    make_branch "fix/auth-redirect"     "fix(auth): redirect"
    make_branch "fix/render-glitch"     "fix: render glitch"
    make_branch "chore/auth-deps"       "chore(auth): bump deps"
    make_branch "chore/typos"           "chore: typo fixes"
    ;;
  f3)
    make_branch "feature/payments"      "feat: payments scaffold"
    echo "DATABASE_URL=postgres://demo:demo@localhost/app" > .env
    git add .env
    git commit --quiet -m "chore: add .env"
    ;;
  *)
    echo "setup.sh: unknown scene: $SCENE" >&2
    exit 1
    ;;
esac

# Per-scene .gct.toml
case "$SCENE" in
  f3)
    cat > .gct.toml <<TOML
[worktree]
dir = "../wt"

[[worktree.post_create]]
type = "copy"
from = ".env"
to = ".env"

[[worktree.post_create]]
type = "command"
command = "echo '✓ env file copied — ready to develop'"
TOML
    ;;
  *)
    cat > .gct.toml <<TOML
[worktree]
dir = "../wt"
TOML
    ;;
esac

git add .gct.toml
git commit --quiet -m "chore: add .gct.toml"

# Stdout: shell snippet that the tape eval's. Sets up env, cd, and the gct
# shell function so the "cd into worktree" action transparently changes
# directory in the recorded shell.
cat <<EOF
export DEMO_FIXTURES="$FIXTURES"
export DEMO_SCENE="$SCENE"
export PATH="$REPO_ROOT/target/release:$TMP/bin:\$PATH"
export PS1='\\W \$ '
cd "$REPO"
eval "\$(gct shell-init bash)"
clear
EOF
