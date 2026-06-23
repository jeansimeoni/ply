# AGENTS.md

## Project Overview

Ply is a local-first package manager and composition engine for coding-agent
assets. It helps developers install, compose, isolate, and manage reusable
agent assets across multiple runtimes without taking ownership of
repository-managed configuration.

Supported targets include:

- Codex
- Claude Code
- Gemini CLI
- Cursor
- future coding-agent ecosystems

The project should feel like:

```txt
git + stow + chezmoi + package manager
```

for coding-agent workflows.

## Core Rules

1. Repositories own their own context.
2. Ply augments existing setups; it does not replace them.
3. All operations should be non-destructive by default.
4. Prefer local-first behavior over network-dependent workflows.
5. Preserve compatibility across multiple agent ecosystems.

If a repository already contains files or directories such as `AGENTS.md`,
`CLAUDE.md`, `.claude/`, `.cursor/`, or `.agents/`, treat them as
repository-owned and out of scope for destructive modification.

## Integration Model

Ply should extend projects through safe composition mechanisms:

- overlays
- generated companion files
- managed blocks
- namespaced assets

Avoid destructive replacement of existing user or repository files.

When merging with existing agent tooling, default to additive behavior and make
ownership boundaries obvious.

## Git and Ignore Policy

Use `.git/info/exclude` for Ply-managed local ignore behavior by default.
Do not modify `.gitignore` unless the user explicitly asks for repository-level
ignore rules.

Rationale:

- `.git/info/exclude` is clone-local
- it avoids polluting repository history
- it is safer for private overlays
- it respects shared repositories and open-source projects

## Namespace Policy

Reserve the `ply-` prefix for Ply-managed assets so they can be identified,
composed, and ignored safely.

Examples:

- `.claude/commands/ply-pr-review.md`
- `.claude/skills/ply-review-diff/`
- `.agents/skills/ply-write-tests/`

Do not create unnamespaced generated assets when a Ply-managed asset is
intended.

## CLI Expectations

The CLI is terminal-oriented and should be pleasant to use without sacrificing
portability or scriptability.

When terminal capabilities allow, support:

- Unicode glyphs
- icons
- progress indicators
- colored output
- structured tables
- spinners

Always maintain a clean ASCII fallback.

## Installer Testing Safety

Treat cargo-dist shell installer testing as a state-changing operation that can
modify the developer's shell startup files.

- A normal `cargo build`, `cargo run`, or direct execution of
  `target/debug/ply` must not be replaced with an installer invocation.
- Never test a Ply installer with `CARGO_HOME` pointing to a temporary
  directory.
- When testing an installer with a temporary destination, always disable shell
  `PATH` modification:

```bash
tmp_dir="$(mktemp -d)"
PLY_INSTALL_DIR="$tmp_dir" \
PLY_NO_MODIFY_PATH=1 \
./ply-installer.sh
```

- Before running any local installer test, verify that its destination is
  stable or that PATH modification is disabled.
- Installer tests must not leave references to `/tmp`, `$TMPDIR`, or another
  ephemeral directory in `~/.profile`, `~/.bashrc`, `~/.bash_profile`,
  `~/.zshrc`, `~/.zshenv`, or `~/.config/fish/conf.d/`.
- Prefer isolated test homes or containers when validating shell startup
  modification behavior.

## Implementation Guidance

- Prefer explicit ownership boundaries over implicit mutation.
- Make additive operations the default path.
- Design for coexistence with repository-owned and user-local assets.
- Keep generated behavior inspectable and reversible.
- Favor Git-native workflows and filesystem transparency.
- Do not assume a single agent runtime or vendor-specific layout.
- Prioritize maintainability and straightforward implementations over cleverness.
- Avoid over-engineered abstractions until they are justified by real use cases.
- When application behavior, supported features, or user-facing workflows
  change, review `README.md` and update it so it reflects the current state of
  the application.

## Commit Message Policy

Commit messages should follow a simple:

```txt
Verb short description
```

Examples:

- `Fix issue with the CLI parsing`
- `Add Git source lockfile support`
- `Refactor package resolution flow`

Do not use commit type prefixes such as:

- `chore:`
- `feat:`
- `fix:`
- `refactor:`

## Decision Filter

When evaluating a feature or code change, prefer the option that best satisfies
these questions:

1. Does it preserve repository ownership of existing agent context?
2. Is it non-destructive and reversible?
3. Does it work as a local-first workflow?
4. Does it compose cleanly with multiple agent runtimes?
5. Does it avoid unnecessary Git pollution?
