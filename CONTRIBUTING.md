# Contributing to Ply

Thanks for contributing to Ply.

## Development Setup

1. Install `mise`.
2. From the repository root, install the pinned Rust toolchain:

```bash
mise install
```

3. Run commands through `mise` if your shell has not already loaded the
   toolchain:

```bash
mise exec -- cargo build
```

## Testing and Quality Checks

Run these before opening a pull request:

```bash
mise exec -- cargo fmt --check
mise exec -- cargo clippy --all-targets -- -D warnings
mise exec -- cargo test --locked
cargo install --locked cargo-dist --version 0.31.0
dist generate --check
dist plan
scripts/check-aur-packaging.sh
```

## Project Expectations

- Keep Ply local-first and non-destructive by default.
- Preserve clear ownership boundaries around repository-managed agent context.
- Prefer additive composition over replacement.
- Keep generated behavior inspectable and reversible.
- Update docs when user-facing workflows or supported packaging channels change.

## Pull Request Expectations

- Use focused pull requests with clear commit messages.
- Include tests for behavior changes.
- Update the README or docs when command behavior, packaging, or release flows change.
- Keep history linear when requested during review.

## Release Process

Releases are maintainer-controlled and are published by the generated `dist`
workflow from protected version tags such as `v0.1.0` or `v0.1.0-rc3`.

Stable package-manager publishing is layered on top of that release flow:

- Homebrew formulas are published to `jeansimeoni/homebrew-tap`.
- AUR metadata is published to `ply-bin` on `aur.archlinux.org`.
- Pre-release tags are not published to Homebrew or AUR.

Before tagging a release:

- ensure `Cargo.toml` has the exact version that matches the release tag
- ensure `CHANGELOG.md` has a heading for that version
- run the local quality gates:

```bash
mise exec -- cargo fmt --check
mise exec -- cargo clippy --all-targets -- -D warnings
mise exec -- cargo test --locked
dist generate --check
dist plan
dist plan --output-format=json --no-local-paths
```

- confirm the plan includes:
  - `ply-installer.sh`
  - `sha256.sum`
  - source tarball
  - `aarch64-apple-darwin`
  - `x86_64-apple-darwin`
  - `x86_64-unknown-linux-musl`
  - `aarch64-unknown-linux-musl`

- build and verify release artifacts:

```bash
dist build --artifacts=all
scripts/verify-release-artifacts.sh target/distrib
```

Downloadable `.deb` and `.rpm` packages are attached by the separate `Native
Linux Packages` workflow after a GitHub Release is published.

To cut a stable release, tag the reviewed release commit and push the tag:

```bash
git tag v0.1.0
git push origin v0.1.0
```

To cut a prerelease, use the matching prerelease version and tag:

```bash
git tag v0.1.0-rc3
git push origin v0.1.0-rc3
```

After the `Release` workflow completes, verify that the GitHub Release contains
archives, checksums, the source tarball, and the shell installer. Then wait for
the `Native Linux Packages` workflow to attach the `.deb` and `.rpm` artifacts.
After both workflows succeed, test the installer from the published release
URL, then run the manual `Native Package Smoke` workflow.

Repository settings should protect `v*` tags so only maintainers can create or
move release tags.

## Packaging Prerequisites

- Create the public GitHub repository `jeansimeoni/homebrew-tap`.
- If GitHub Actions cannot create releases with `GITHUB_TOKEN`, add
  `RELEASE_TOKEN` to the `ply` repository secrets with `repo` access so the
  release workflow can create and update GitHub Releases.
- Add `HOMEBREW_TAP_TOKEN` to the `ply` repository secrets with write access
  to that tap repository.
- Create a dedicated AUR SSH key for GitHub Actions and add the public key to
  your AUR account.
- Add `AUR_SSH_PRIVATE_KEY` and `AUR_KNOWN_HOSTS` to the `ply` repository
  secrets.
- Optionally set repository variables `AUR_PACKAGER_NAME` and
  `AUR_PACKAGER_EMAIL` to control the Git identity used for AUR commits.

## First AUR Bootstrap

1. Generate `PKGBUILD` and `.SRCINFO` from the release `sha256.sum` file with
   `scripts/generate-aur-ply-bin.sh`, including the maintainer identity.
2. Clone `ssh://aur@aur.archlinux.org/ply-bin.git` with `master` as the local
   default branch.
3. Copy in the generated `PKGBUILD`, `.SRCINFO`, and
   `packaging/aur/ply-bin/LICENSE`.
4. Commit and push the initial `master` branch once.

## Licensing

By submitting contributions, you agree that your work is licensed under the
project license and can be redistributed under those terms.
