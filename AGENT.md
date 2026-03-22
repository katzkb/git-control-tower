# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Git Control Tower (gct) — a terminal TUI tool that acts as a "control tower" for Git/GitHub workflows. It provides full-screen views for Log, PR, Branch, and Worktree management, enabling developers to oversee repositories, start reviews, and clean up branches entirely from the terminal.

## Tech Stack

- **Language:** Rust
- **TUI:** ratatui + crossterm
- **Async runtime:** tokio
- **External dependencies:** `git` CLI and `gh` (GitHub CLI) must be installed and authenticated
- **Platforms:** macOS, Windows, Linux

## Build & Development Commands

```bash
cargo build              # Build
cargo run                # Run the TUI
cargo test               # Run all tests
cargo test <test_name>   # Run a single test
cargo clippy             # Lint
cargo fmt                # Format code
cargo fmt -- --check     # Check formatting without modifying
```

## Architecture

The application has a branch-centric 2-pane layout:

- **Main View** — Left sidebar lists branches with filter modes (Local/My PR/Review). Right detail pane shows git status, worktree info, and PR details with markdown rendering.
- **Log View** — Git commit history, accessible via `l` key.

Key design principles:
- All Git operations go through `git` CLI; all GitHub operations go through `gh` CLI (not direct API calls)
- Destructive operations (branch deletion, worktree removal) must always have a confirmation step
- GitHub API calls run async (tokio) so the UI stays responsive during network I/O
- Navigation is keyboard-only

## Language

- Comments and code should be in English.
- All PR titles, descriptions, commit messages, and issue content must be written in English.
