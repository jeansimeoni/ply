# Security Policy

## Reporting A Vulnerability

If you believe you have found a security issue in Ply, do not open a public
GitHub issue with exploit details.

Report it privately to the maintainer first. Until a dedicated project inbox is
published, use GitHub Security Advisories for private disclosure when possible.

Include:

- the affected version
- the environment or platform involved
- clear reproduction steps
- impact assessment if you have one

## Scope

Security reports are most useful for issues that affect:

- filesystem safety
- destructive or ownership-boundary bypasses
- credential or secret exposure
- unsafe package resolution or execution behavior
- release artifact or installer integrity

## Supported Releases

The current supported public baseline is:

- `v0.1.1`
