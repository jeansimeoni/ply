# Changelog

## Unreleased

## v0.1.2

- allow package repositories to contain top-level agent workspace directories
  such as `.claude/`, `.agents/`, and `.codex/` as private authoring state
- prevent the shell installer from persisting temporary `CARGO_HOME` paths in
  shell startup files by installing to `$HOME/.local/bin`

## v0.1.1

- add native support for resolving shared configuration from the main Git
  worktree while keeping generated state isolated per worktree
- show the resolved configuration root and active worktree in project command
  reports
- resolve clone-local ignore rules correctly when `.git` is a worktree pointer
  file
- improve Codex command metadata rendering and prompt-resource parse errors

## v0.1.0

First public release of Ply.

Highlights:

- local-first package management for coding-agent assets
- additive composition across Codex and Claude Code
- Git and path sources with locked revisions in `ply.lock`
- deterministic managed output generation under `.ply/generated/`
- release automation for GitHub Releases, Homebrew, native Linux packages, and AUR
- public installation and contributor documentation
