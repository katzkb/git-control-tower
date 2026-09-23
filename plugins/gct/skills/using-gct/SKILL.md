---
name: using-gct
description: Use when creating or switching to a git worktree, starting parallel work on another branch, setting up a separate working directory for a branch or PR, or cleaning up merged branches, in repositories where the gct CLI is installed (a `.gct.toml` file or `command -v gct` succeeding are the signals). Applies before running raw `git worktree add` or bulk branch deletion.
---

# Using gct

gct (Git Control Tower) manages git worktrees by convention. When gct is
available, never run `git worktree add` yourself: `gct wt` applies the
project's configured worktree path layout (`worktree.dir`, `{repo}` token)
and runs its post-create hooks (copying `.env` files, setup commands), so
the branch↔directory mapping stays consistent with the user's other
worktrees and the new worktree is immediately usable. A worktree created
with raw git lands in an unconventional place and silently skips those
hooks.

## Availability

Use gct only if `command -v gct` succeeds. If it is not installed, fall
back to plain git — do not install gct on your own.

## Quick reference

| Command | Purpose | stdout on success |
|---|---|---|
| `gct wt <branch>` | Reuse or create the branch's worktree (runs hooks) | the worktree path, nothing else |
| `gct cd <branch>` | Path of an existing worktree | the worktree path; exit 1 and no output if none exists |
| `gct ls` | List worktrees | `branch<TAB>path` per line (`(detached)` for detached HEAD) |
| `gct ls branches` | List local branches | one branch name per line |
| `gct prune [--yes] [--force]` | Delete merged branches and their worktrees | human-readable report, never a bare path |

Errors go to stderr; `wt` and `cd` print nothing to stdout on failure, so
command substitution is safe.

## Working on a branch (the core workflow)

```bash
wt=$(command gct wt feature/x)   # create or reuse the worktree; prints its path
cd "$wt"
```

- Use `command gct` (not bare `gct`) for `wt` and `cd` in scripts and
  non-interactive shells: gct's optional shell integration redefines
  `gct` as a shell function that cd's to the path instead of printing
  it, which leaves command substitution empty. `command gct` always
  invokes the binary, which prints the path.
- Idempotent: if the worktree already exists, `gct wt` just prints its
  path. Safe to re-run.
- On first creation the printed path may contain `..` segments; it is
  valid for `cd` as is.
- `Warning: post-create hook failed: ...` on stderr is non-fatal (exit
  stays 0): the worktree is usable — report the warning to the user
  instead of aborting.
- Run `gct wt` from the repository's primary checkout, not from inside
  another linked worktree: the configured layout and hooks resolve
  relative to the current working tree's top level.

### New branches: create the branch first

`gct wt` checks out existing branches — local ones directly, others
fetched from origin. It does not create new branches; asking for one
fails with `Error: failed to fetch '<branch>' from origin`. That error
means the branch does not exist yet, not that gct is broken. Create the
branch, then make its worktree:

```bash
git branch bugfix/header main
cd "$(command gct wt bugfix/header)"
```

## Cleaning up merged branches

`gct prune` deletes local branches already merged into the default
branch, together with their worktrees (worktree first, so deletion never
fails on a checked-out branch). The current branch and protected branches
(the configured `protected_branches`) are always kept.

It is safe by default — without `--yes` it is a dry run:

```bash
gct prune          # dry run: lists candidates, deletes nothing
gct prune --yes    # actually delete
```

Show the user the dry-run output and get their confirmation before
running `--yes`. `--force` (combined with `--yes`) uses
`git worktree remove --force` and `git branch -D` — needed for
worktrees with untracked or modified files; treat it as more
destructive and confirm it separately.

Squash- and rebase-merged branches are detected through `gh` (the
branch's PR is merged on GitHub), so they do not need `--force`:

- Listed as `<branch> (squash-merged PR #N)` and deleted with
  `git branch -D`, but only when the local tip equals the merged PR head.
- `Skipped <branch>: local commits not in merged PR #N` means the
  branch has commits after the merge; it is kept. Tell the user rather
  than deleting it yourself.
- `Note: squash-merged branches were not checked (<reason>)` on stderr
  means `gh` is missing, unauthenticated, or origin is not a GitHub
  remote. Exit code stays 0 and only branches merged into the default
  branch are pruned.

`gct prune` never deletes remote branches (`git push --delete`) — do
not do that unless the user explicitly asks for it.

## MCP alternative

If the gct MCP server is configured for the agent (`gct mcp` in the MCP
client config), prefer its tools — `create_worktree`, `list_worktrees`,
`list_branches` — over shelling out. They run the same engine as
`gct wt` / `gct ls` and return structured results.

## When gct does not apply

- gct is not installed → plain git, as usual.
- Removing one specific (possibly unmerged) worktree →
  `git worktree remove`; `gct prune` only handles merged branches in
  bulk.
- `git worktree move` / `lock` / `repair` → raw git; gct has no
  equivalent.
- Never launch bare `gct` (the TUI) from an agent session: it is an
  interactive full-screen app that needs a TTY and the GitHub CLI. The
  subcommands above are the non-interactive surface.
