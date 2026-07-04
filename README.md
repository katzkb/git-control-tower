# Git Control Tower (gct)

A terminal UI tool that acts as a "control tower" for Git/GitHub workflows. Oversee your repository, start PR reviews, and clean up branches — all from the terminal.

![Hero demo: pick a review-requested PR, drop it into a local worktree, then open it in any tool.](docs/images/hero.gif)

## Features

- **Branch-centric 2-pane view** — Left sidebar lists branches with PR status, review indicators, and worktree info. Right pane shows git status, PR details with markdown rendering.
- **Filter modes** — Switch between Local branches (`1`), your PRs (`2`), and review-requested PRs (`3`). Toggle merged PRs with `m` and team reviews with `t`.
- **Search** — Press `/` to filter branches by name. Matches are highlighted. Press `Enter` to keep the filter active.
- **Review status** — Color-coded review indicators: needs review (red), approved (green), changes requested (yellow).
- **Action menu** — Press `Enter` to open a context-sensitive menu: copy branch name and open PR in browser, plus actions grouped into Worktree (go to / create / delete) and Branch (create from this / delete) sections.
- **Worktree management** — Create worktrees from branches or PRs. Auto-run post-create hooks (copy files, create symlinks, run commands). Force delete worktrees with untracked files.
- **Branch cleanup** — Multi-select branches with `Space`, select all merged with `a`, batch delete with `d`. Force deletes squash-merged branches.
- **Commit log** — View commit history with `l`.
- **Verbose mode** — Run with `--verbose` to surface silenced errors for troubleshooting.

## In motion

**Branch cleanup** — toggle merged PRs, select all merged, batch delete.

![Branch cleanup demo](docs/images/f1-cleanup.gif)

**Search & filter** — narrow the sidebar with `/`, then jump between filter modes.

![Search and filter demo](docs/images/f2-search.gif)

**Worktree post-create hooks** — `.gct.toml` actions (file copy, symlinks, commands) fire automatically when a worktree is created.

![Worktree hooks demo](docs/images/f3-hooks.gif)

**Cross-repo Reviews** — `Review` mode aggregates PRs across every repo you're involved in; the sidebar groups by repo and the action menu hints at uncloned repos.

![Cross-repo Reviews demo](docs/images/f4-cross-repo.gif)

## Requirements

- [git](https://git-scm.com/) CLI
- [gh](https://cli.github.com/) CLI (authenticated)
- A terminal with 256-color support

## Installation

### Homebrew (macOS / Linux)

```bash
brew install katzkb/tap/gct
```

Upgrade:

```bash
brew update && brew upgrade katzkb/tap/gct
```

Supported platforms: macOS (Apple Silicon / Intel), Linux (x86_64).

### From source

```bash
git clone https://github.com/katzkb/git-control-tower.git
cd git-control-tower
cargo install --path .
```

## Setup

To enable the "cd into worktree" feature, add the following to your shell configuration:

```bash
# zsh (~/.zshrc)
eval "$(gct shell-init zsh)"

# bash (~/.bashrc)
eval "$(gct shell-init bash)"

# fish (~/.config/fish/config.fish)
gct shell-init fish | source
```

This wraps `gct` with a shell function that captures the worktree path and runs `cd` in your shell. The same wrapper also powers the `gct cd <branch>` command (see [Usage](#usage)).

## Usage

```bash
# Run inside a git repository
gct

# Show version
gct --version

# Enable verbose error output
gct --verbose

# Jump straight into an existing worktree (no TUI)
gct cd feature/login

# Create (or reuse) a worktree for a branch, then cd into it
gct wt feature/login

# List worktrees or branches (plain text, for scripting)
gct ls              # worktrees, as `branch<TAB>path`
gct ls branches

# Delete merged branches and their worktrees (dry-run by default)
gct prune           # show what would be deleted
gct prune --yes     # actually delete

# Shell completion
eval "$(gct completions zsh)"
```

These subcommands run without launching the TUI:

- **`gct cd <branch>`** — print the path of the worktree checked out for `<branch>`
  and cd into it via the shell wrapper. Requires the shell integration from
  [Setup](#setup). If no worktree exists it exits non-zero and prints nothing —
  use `gct wt` to create one.
- **`gct wt <branch>`** — the create-side complement to `cd`: reuse the branch's
  worktree or create one (applying the `post_create` hooks from your config), then
  cd into it. Also requires the shell integration.
- **`gct ls [worktrees|branches]`** — plain-text listing for scripting, e.g.
  ``gct cd "$(gct ls | fzf | cut -f1)"``. Worktrees print as `branch<TAB>path`.
- **`gct prune [--dry-run] [--yes] [--force]`** — delete merged branches and their
  worktrees (protected and current branches are skipped). Lists candidates only
  unless `--yes` is given; `--force` uses `worktree remove --force` and `branch -D`.

> `cd` and `wt` emit a bare worktree path that the shell wrapper cd's into; `ls`
> and `prune` print informational output and never change your directory.

## Keybindings

### Global

| Key | Action |
|-----|--------|
| `1` | Filter: Local branches |
| `2` | Filter: My PRs |
| `3` | Filter: Review requested |
| `l` | Log view |
| `?` | Help |
| `q` | Quit |

### Main View

| Key | Action |
|-----|--------|
| `j` / `k` | Navigate sidebar |
| `/` | Search branches |
| `Enter` | Action menu |
| `Space` | Toggle branch selection |
| `a` | Select all merged branches |
| `w` | Create worktree |
| `d` | Delete selected branches / worktree |
| `m` | Toggle merged PRs (My PR / Review) |
| `t` | Toggle team reviews (Review only) |

### Log View

| Key | Action |
|-----|--------|
| `j` / `k` | Navigate commits |
| `Esc` | Back to main view |

## Configuration

gct deep-merges every config file it finds, from lowest to highest priority:

1. `~/.gct.toml` (global, lowest priority)
2. `~/.config/gct/config.toml` (global)
3. `.gct.toml` in the repository root (project-local, highest priority)

Higher-priority files override lower ones **key by key**: nested tables
(`[worktree]`, `[workspace]`) are merged, so a project-local `.gct.toml` can
override individual settings while inheriting the rest from your global config.
Scalars and arrays (e.g. `worktree.dir`, `protected_branches`) are replaced
wholesale by the highest-priority file that sets them — with one exception:
`worktree.post_create` hooks are **additive** (like `.gitignore` layering).
Hooks from every config file run, global layers first, then the
project-local ones, so a global "copy `.env`" hook and a project-local
"run `npm ci`" hook both fire.

To opt out of an inherited hook (the equivalent of a `.gitignore` `!`
pattern), give the hook a `name` in the lower-priority file and list it in
`disable_post_create` in the higher-priority one:

```toml
# ~/.gct.toml — global hook, named so projects can opt out
[[worktree.post_create]]
name = "copy-env"
type = "copy"
from = ".env"
to = ".env"
```

```toml
# .gct.toml in a repo that doesn't want the global copy-env hook
[worktree]
disable_post_create = ["copy-env"]
```

Unnamed hooks always run; `disable_post_create` only filters hooks inherited
from lower-priority files, never the file's own hooks.

For example, with a global `~/.config/gct/config.toml`:

```toml
[workspace]
clone_root = "~/workspace"

[worktree]
dir = "../wt/{repo}"
```

a repo-specific `.gct.toml` containing only:

```toml
[worktree]
dir = "../custom"
```

uses `../custom` for that repo's worktrees while still inheriting `clone_root`
from the global config. Project-local config is also useful for per-repo
worktree hooks (e.g. copying `.env`, running `npm ci`).

### Cross-repo Reviews

`My PR` (`2`) and `Review` (`3`) modes show pull requests across **all** of your GitHub repositories, not just the one you launched gct from. When the result spans multiple repos, the sidebar groups branches under repo headers; otherwise it stays flat as before.

For cross-repo PRs, `Create Worktree` and `cd into Worktree` operate on the local clone of that repo. gct discovers the clone path one of two ways:

1. **Auto-inferred from your active repo's path.** If you launched gct from `~/workspace/github.com/owner/name`, gct strips the `<host>/<owner>/<name>` suffix and treats `~/workspace` as the clone root. ghq users get this for free.

2. **Configured explicitly** when auto-inference fails. Add to `.gct.toml` or `~/.config/gct/config.toml`:

   ```toml
   [workspace]
   clone_root = "~/workspace"
   ```

   gct then expects clones at `<clone_root>/<host>/<owner>/<name>`.

If neither path resolves, cross-repo entries remain visible but `Create Worktree` is greyed out with a one-line hint pointing at the expected clone location.

**Per-repo config for cross-repo worktrees:** when you create a worktree for another repo, gct applies that **target repo's** `.gct.toml` (merged on top of your global config), not the `.gct.toml` of the repo you launched gct from. So each repo's `worktree.dir` and `post_create` hooks take effect even when created from elsewhere.

**Cross-host coverage:** My PR / My Review fan out across every host present in the repos gct has discovered locally (origin remotes), so a workspace mixing github.com clones with GitHub Enterprise clones surfaces PRs from both. Hosts you are authenticated to but have never cloned from are skipped — clone any one repo from that host (or `cd` into one) so gct can pick it up.

**Limitations (v1):** Bulk delete (`d` after Space-selection) still operates on active-repo branches only.

### Worktree Settings

```toml
[worktree]
# Base directory for new worktrees (default: "..")
# Branch name becomes the subdirectory: feature/auth → ../feature/auth
#
# The `{repo}` token expands to the repository name, so a single global
# config can keep every repo's worktrees apart, e.g.:
#   dir = "../wt/{repo}"   # feature/auth → ../wt/<repo-name>/feature/auth
# A `dir` without `{repo}` behaves exactly as before (backward compatible).
dir = ".."

# Post-create hooks run automatically after worktree creation.
# Errors are non-fatal — the worktree is created even if hooks fail.

# Copy files from the main worktree
[[worktree.post_create]]
type = "copy"
from = ".env"
to = ".env"

# Create symlinks to shared directories
[[worktree.post_create]]
type = "symlink"
from = "node_modules"
to = "node_modules"

# Run shell commands in the new worktree
[[worktree.post_create]]
type = "command"
command = "npm ci"
```

### Protected Branches

Protected branches are excluded from the `[merged]` label, yellow name color, and all deletion actions (`space`, `a`, action menu). Default: `["main", "master", "develop"]`.

```toml
# Override the default list — useful if your team uses `staging`, `release`, etc.
protected_branches = ["main", "develop", "staging"]

# Or disable protection entirely
# protected_branches = []
```

Branch names are matched case-sensitively.

## Regenerating the demo GIFs

The README's GIFs are produced by [VHS](https://github.com/charmbracelet/vhs) against a fully reproducible local fixture (a fresh git repo + a `gh` shim that returns canned JSON). No GitHub account is needed.

```bash
brew install vhs        # one-time
make demos              # rebuild gct + re-record all five GIFs
make demos-hero         # one scene at a time
```

See `scripts/demo/` for the tape files, fixtures, and setup script. When a new `gh` invocation is added to gct, extend `scripts/demo/gh-stub` accordingly so the recording stays self-contained.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
