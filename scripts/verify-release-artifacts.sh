#!/usr/bin/env sh
set -eu

artifact_dir="${1:-target/distrib}"

linux_targets="x86_64-unknown-linux-musl aarch64-unknown-linux-musl"
macos_targets="x86_64-apple-darwin aarch64-apple-darwin"
all_targets="$linux_targets $macos_targets"
required_files="LICENSE README.md CHANGELOG.md"

fail() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

info() {
    printf '%s\n' "$*"
}

command_exists() {
    command -v "$1" >/dev/null 2>&1
}

target_archive() {
    printf '%s/ply-%s.tar.xz' "$artifact_dir" "$1"
}

require_archive() {
    target="$1"
    archive="$(target_archive "$target")"
    [ -f "$archive" ] || fail "missing archive: $archive"
}

extract_archive() {
    target="$1"
    dest="$2"
    archive="$(target_archive "$target")"
    tar -xJf "$archive" -C "$dest"
}

find_in_tree() {
    root="$1"
    name="$2"
    find "$root" -type f -name "$name" -print -quit
}

host_target() {
    os="$(uname -s 2>/dev/null || printf unknown)"
    arch="$(uname -m 2>/dev/null || printf unknown)"

    case "$os:$arch" in
        Linux:x86_64|Linux:amd64) printf 'x86_64-unknown-linux-musl' ;;
        Linux:aarch64|Linux:arm64) printf 'aarch64-unknown-linux-musl' ;;
        *) printf 'unsupported' ;;
    esac
}

verify_common_contents() {
    target="$1"
    root="$2"

    binary="$(find_in_tree "$root" ply)"
    [ -n "$binary" ] || fail "$target archive does not contain ply binary"

    for file in $required_files; do
        path="$(find_in_tree "$root" "$file")"
        [ -n "$path" ] || fail "$target archive does not contain $file"
    done

    printf '%s' "$binary"
}

verify_linux_linkage() {
    target="$1"
    binary="$2"

    command_exists file || fail "file command is required for Linux artifact verification"
    file_output="$(file "$binary")"
    info "[$target] file: $file_output"

    case "$file_output" in
        *ELF*) ;;
        *) fail "$target binary is not reported as ELF" ;;
    esac

    if command_exists ldd; then
        set +e
        ldd_output="$(ldd "$binary" 2>&1)"
        ldd_status=$?
        set -e
        info "[$target] ldd status: $ldd_status"
        printf '%s\n' "$ldd_output" | sed "s/^/[$target] ldd: /"
    else
        info "[$target] ldd not found; skipped dynamic dependency inspection"
    fi
}

verify_host_execution() {
    target="$1"
    binary="$2"
    expected_host="$(host_target)"

    if [ "$target" != "$expected_host" ]; then
        info "[$target] not host-compatible for execution on $(uname -s)/$(uname -m); skipped -V/--version/--help"
        return 0
    fi

    chmod +x "$binary"
    "$binary" -V >/dev/null
    "$binary" --version >/dev/null
    "$binary" --help >/dev/null
    info "[$target] -V, --version, and --help passed"
}

[ -d "$artifact_dir" ] || fail "artifact directory does not exist: $artifact_dir"
command_exists tar || fail "tar command is required"
command_exists find || fail "find command is required"

for target in $all_targets; do
    require_archive "$target"
done

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT HUP INT TERM

for target in $all_targets; do
    info "verifying $target"
    target_dir="$work_dir/$target"
    mkdir -p "$target_dir"
    extract_archive "$target" "$target_dir"
    binary="$(verify_common_contents "$target" "$target_dir")"

    case "$target" in
        *-unknown-linux-musl)
            verify_linux_linkage "$target" "$binary"
            verify_host_execution "$target" "$binary"
            ;;
        *-apple-darwin)
            info "[$target] archive content verified; launch must be checked on macOS"
            ;;
        *)
            fail "unexpected target: $target"
            ;;
    esac
done

info "release artifact verification passed"
