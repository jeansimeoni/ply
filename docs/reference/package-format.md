# Package Format And Asset Kinds

This reference describes the on-disk format of a Ply package.

## Package root

Each package root must contain:

```txt
ply-package.toml
```

Minimal example:

```toml
name = "review-tools"
```

Supported manifest fields:

- `name` (required)
- `version` (optional, must be valid semver if present)
- `description` (optional)
- `license` (optional)
- `targets` (optional)

Each configured source points at one package root. A source is invalid if
`ply-package.toml` is missing from the source root.

If package-level `targets` is present, it defines the maximum adapter set for
the package. Resource-level `targets` may only narrow further inside that set.

Package validation also rejects:

- unsupported adapter-owned directories such as `.claude/`, `.agents/`,
  `.codex/`, `.cursor/`, and `.gemini/`
- package roots that contain no supported managed assets

## Supported top-level asset kinds

Ply currently recognizes these package-level asset kinds:

- `commands/`
- `skills/`
- `agents/`
- `rules/`
- `hooks/`
- `output-styles/`
- `local-instructions.md`

Directory-based kinds:

- `commands`
- `skills`
- `agents`
- `rules`
- `hooks`
- `output-styles`

File-based kind:

- `local-instructions`

## Required filenames by kind

Some kinds use a required primary Markdown filename inside each resource:

- `skills/<name>/SKILL.md`
- `agents/<name>/AGENT.md`

`commands/` and `output-styles/` use Markdown files directly in their kind
directory.

Example:

```txt
review-tools/
├── ply-package.toml
├── commands/
│   └── pr-review.md
├── agents/
│   └── reviewer/
│       └── AGENT.md
└── skills/
    └── review-diff/
        └── SKILL.md
```

A package root that contains only `ply-package.toml` is not a valid
consumable package. Add at least one supported asset kind such as `skills/`,
`commands/`, `agents/`, `rules/`, `hooks/`, `output-styles/`, or
`local-instructions.md`.

## Prompt-resource kinds

These kinds share Ply's YAML frontmatter authoring model:

- `commands`
- `skills`
- `agents`
- `output-styles`

`rules` and `hooks` are native adapter resources and do not use this shared
frontmatter model.

## Shared frontmatter keys

Supported shared keys:

- `name`
- `description`
- `category`
- `argument-hint` or `argument_hint`
- `keep-coding-instructions` or `keep_coding_instructions`

Restrictions:

- `argument-hint` is only valid for `commands`.
- `keep-coding-instructions` is only valid for `output-styles`.
- Quote `argument-hint` values when they contain bracketed placeholders or
  CLI-like syntax such as `[topic]`, `<path>`, or `--flag=value`.

Example:

```md
---
name: technical-writer
description: Write clear technical documentation
---

Write accurate documentation with verifiable examples.
```

Command example with multiple arguments:

```md
---
name: review-pr
description: Review a pull request
argument-hint: "<ticket-number> [--coverage=80] [--post-comments]"
---

Interpret `$ARGUMENTS` as:

- required: `<ticket-number>`
- optional: `--coverage=<n>`
- optional: `--post-comments`
```

## Claude frontmatter keys

Supported `claude:` keys:

- `model`
- `tools`
- `argument-hint` or `argument_hint`
- `disable-model-invocation` or `disable_model_invocation`
- `context`
- `agent`
- `user-invocable` or `user_invocable`
- `disallowedTools` or `disallowed_tools`
- `mcpServers` or `mcp_servers`

Restrictions:

- `context`, `agent`, `disable-model-invocation`, and `user-invocable` are only
  valid for `commands` and `skills`.
- `disallowedTools` and `mcpServers` are only valid for `agents`.

## Codex frontmatter keys

Supported `codex:` keys:

- `model`
- `reasoning_effort` or `reasoning-effort`
- `tools`
- `sandbox_mode` or `sandbox-mode`
- `approval_policy` or `approval-policy`
- `mcp_servers` or `mcp-servers`
- `interface`
- `policy`
- `dependencies`

Restrictions:

- `sandbox_mode`, `approval_policy`, and `mcp_servers` are only valid for
  `agents`.
- `interface`, `policy`, and `dependencies` are only valid for `skills`.

Unknown frontmatter keys are rejected.

## Per-resource adapter targeting

Resources target all adapters allowed by the package unless you add metadata
that limits them.

If `ply-package.toml` defines package-level `targets`, that list is an upper
bound. Resource-level `targets` can narrow to a subset such as `["codex"]`,
but they cannot introduce an adapter the package itself does not allow.

Use:

- `skills/<name>/ply-asset.toml` for directory resources
- `<file>.ply-asset.toml` for file resources

Example:

```toml
targets = ["codex"]
```

If package-level `targets` is omitted or empty, the package applies to all
enabled adapters. If resource-level `targets` is omitted or empty, that
resource inherits the package adapter set.

## Naming and exposed prefixes

Package authors use natural names in the package root. When Ply writes into
adapter-owned namespaces, it adds the `ply-` prefix where required.

Examples:

- `skills/review-diff/` becomes `.claude/skills/ply-review-diff/`
- `agents/reviewer/` becomes `.claude/agents/ply-reviewer/`
- `commands/pr-review.md` becomes `.agents/commands/ply-pr-review.md`

## Adapter outputs

Current adapter mapping:

Claude:

- `commands` -> `.claude/commands/`
- `skills` -> `.claude/skills/`
- `agents` -> `.claude/agents/`
- `local-instructions` -> `CLAUDE.local.md`
- `rules` -> `.claude/rules/`
- `hooks` -> `.claude/hooks/`
- `output-styles` -> `.claude/output-styles/`

Codex:

- `commands` -> `.agents/commands/`
- `skills` -> `.agents/skills/`
- `agents` -> `.codex/agents/*.toml`
- `local-instructions` -> `AGENTS.override.md`
- `rules` -> `.codex/rules/`
- `hooks` -> `.codex/hooks/` plus `.codex/hooks.json`
- `output-styles` -> `AGENTS.override.md`

## Generated adapter-specific behavior

For shared prompt resources, Ply renders adapter-specific output from the
authored Markdown:

- Claude receives adapter-native Markdown where applicable.
- Codex skills receive generated `SKILL.md` frontmatter and may also receive
  generated `agents/openai.yaml` when `codex.interface`, `codex.policy`, or
  `codex.dependencies` are present.
- Codex agents receive generated `.codex/agents/<name>.toml`.
- Codex commands and Codex output styles can receive a generated Markdown
  settings preamble based on Codex metadata.

Package authors must not author `agents/openai.yaml` directly inside a skill.
The adapter-specific rendering details below explain the exact output shape for
Codex, Claude, and additive managed files.

## How Ply renders each adapter

Ply treats the package Markdown as the source of truth, then renders the
closest shape each adapter expects.

### Codex

Use Codex frontmatter when the target expects structured config rather than a
plain Markdown prompt.

- `agents/<name>/AGENT.md` becomes `.codex/agents/ply-<name>.toml`.
- The Markdown body becomes `developer_instructions` in that TOML file.
- Supported Codex metadata such as `model`, `reasoning_effort`,
  `sandbox_mode`, `approval_policy`, and `mcp_servers` become TOML fields.
- `skills/<name>/SKILL.md` becomes `.agents/skills/ply-<name>/SKILL.md`, and
  `codex.interface`, `codex.policy`, or `codex.dependencies` also generate
  `.agents/skills/ply-<name>/agents/openai.yaml`.
- `commands/*.md` stay Markdown in `.agents/commands/`, but Codex metadata is
  rendered as a `## Ply Command Metadata` and `## Ply Codex Settings` preamble
  ahead of the prompt body when applicable.

Example:

```md
---
name: reviewer
codex:
  model: gpt-5.5
  reasoning_effort: high
  sandbox_mode: workspace-write
  approval_policy: on-request
---

Review carefully and surface findings first.
```

renders to a `.codex/agents/ply-reviewer.toml` file with fields such as:

```toml
name = "reviewer"
developer_instructions = "Review carefully and surface findings first."
model = "gpt-5.5"
model_reasoning_effort = "high"
sandbox_mode = "workspace-write"
approval_policy = "on-request"
```

For Codex commands, Ply keeps the body as Markdown and injects command metadata
as prompt text rather than a separate structured command schema.

Example:

```md
---
name: review-pr
description: Review a pull request
argument-hint: "<ticket-number> [--coverage=80] [--post-comments]"
codex:
  model: gpt-5.5
  tools:
    - shell
    - patch
---

Interpret `$ARGUMENTS` as:

- required: `<ticket-number>`
- optional: `--coverage=<n>`
- optional: `--post-comments`
```

renders to:

```md
## Ply Command Metadata

Name: review-pr
Description: Review a pull request
Arguments: <ticket-number> [--coverage=80] [--post-comments]

---

## Ply Codex Settings

Preferred model: gpt-5.5
Preferred tools: shell, patch

---

Interpret `$ARGUMENTS` as:

- required: `<ticket-number>`
- optional: `--coverage=<n>`
- optional: `--post-comments`
```

### Claude

Claude keeps the prompt resource as Markdown and rewrites the frontmatter into
Claude's expected keys.

- `commands/*.md` become `.claude/commands/ply-*.md`.
- `skills/<name>/SKILL.md` become `.claude/skills/ply-<name>/SKILL.md`.
- `agents/<name>/AGENT.md` become `.claude/agents/ply-<name>/AGENT.md`.
- `output-styles/*.md` become `.claude/output-styles/ply-*.md`.

In those files, Ply preserves the prompt body and renders Claude metadata into
Markdown frontmatter such as `allowed-tools`, `model`, `agent`, `context`, or
`keep-coding-instructions` when the resource kind supports them.

### Additive prompt files

Some resources do not have a dedicated native target file for every adapter.
When that happens, Ply surfaces the prompt content through additive managed
files instead of asking you to author adapter-owned files directly.

- Claude `local-instructions.md` is injected into a managed block inside
  `CLAUDE.local.md`.
- Codex `local-instructions.md` and Codex `output-styles/*.md` are composed
  into `AGENTS.override.md`.
- `AGENTS.override.md` starts with a generated notice, includes the current
  repo-owned `AGENTS.md` if it exists, then appends Ply-managed sections for
  local instructions and output styles.

If a resource does not map cleanly to a standalone Codex config file, Ply
surfaces it as prompt content in that additive override file.

This is the same path Ply uses when a package includes content that an adapter
does not support as a first-class standalone asset. Rather than drop it or ask
you to hand-author adapter-owned files, Ply promotes that content into the
managed additive prompt file for the target adapter.

## Related docs

- [Create your first package](../guides/create-package.md)
- [Configuration and layering](configuration-and-layering.md)
