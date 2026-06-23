#!/usr/bin/env sh
set -eu

repo_root="$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd)"
dist_config="$repo_root/dist-workspace.toml"
install_docs="$repo_root/docs/install.md"

expected='install-path = "~/.local/bin"'

grep -Fxq "$expected" "$dist_config" || {
    printf 'error: the shell installer must use a stable, user-owned install path\n' >&2
    printf 'expected %s in %s\n' "$expected" "$dist_config" >&2
    exit 1
}

if grep -Fq '${CARGO_HOME:-$HOME/.cargo}/bin' "$install_docs"; then
    printf 'error: install documentation still advertises the unsafe CARGO_HOME-derived path\n' >&2
    exit 1
fi

printf 'shell installer safety check passed\n'
