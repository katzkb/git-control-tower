# Git Control Tower (gct)

A terminal UI tool that acts as a "control tower" for Git/GitHub workflows. Oversee your repository, start PR reviews, and clean up branches — all from the terminal.

## Features

- **Branch-centric 2-pane view** — Left sidebar lists branches with PR and worktree indicators. Right pane shows git status, worktree path, and PR details with markdown rendering.
- **Filter modes** — Switch between Local branches (`1`), your PRs (`2`), and review-requested PRs (`3`).
- **Worktree management** — Create worktrees from PRs with `w` for instant code review. Delete with `d`.
- **Branch cleanup** — Multi-select branches with `Space`, select all merged with `a`, batch delete with `d`.
- **Commit log** — View commit history with `l`.

## Requirements

- [git](https://git-scm.com/) CLI
- [gh](https://cli.github.com/) CLI (authenticated)
- A terminal with 256-color support

## Installation

```bash
# From source
git clone https://github.com/katzkb/git-control-tower.git
cd git-control-tower
cargo install --path .
```

## Usage

```bash
# Run inside a git repository
gct
```

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
| `Space` | Toggle branch selection |
| `a` | Select all merged branches |
| `d` | Delete selected branches / worktree |
| `w` | Create worktree from PR |

### Log View

| Key | Action |
|-----|--------|
| `j` / `k` | Navigate commits |
| `Esc` | Back to main view |

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
