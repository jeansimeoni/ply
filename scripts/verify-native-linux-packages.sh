#!/usr/bin/env bash
set -euo pipefail

artifact_dir="${1:-target/distrib}"

fail() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

info() {
    printf '%s\n' "$*"
}

[[ -d "$artifact_dir" ]] || fail "artifact directory does not exist: $artifact_dir"
command -v dpkg-deb >/dev/null 2>&1 || fail "dpkg-deb is required"

deb_files=("$artifact_dir"/ply_*.deb)
if [[ ! -e "${deb_files[0]}" ]]; then
    fail "no .deb packages found in $artifact_dir"
fi

for deb_file in "${deb_files[@]}"; do
    info "verifying $(basename "$deb_file")"
    dpkg-deb --info "$deb_file" >/dev/null
    listing="$(dpkg-deb --contents "$deb_file")"
    grep -q "./usr/bin/ply" <<<"$listing" || fail "$(basename "$deb_file") does not contain /usr/bin/ply"
    grep -q "./usr/share/doc/ply/README.md" <<<"$listing" || fail "$(basename "$deb_file") does not contain README.md"
    grep -q "./usr/share/doc/ply/CHANGELOG.md" <<<"$listing" || fail "$(basename "$deb_file") does not contain CHANGELOG.md"
    grep -q "./usr/share/licenses/ply/LICENSE" <<<"$listing" || fail "$(basename "$deb_file") does not contain LICENSE"
done

mapfile -t rpm_files < <(find "$artifact_dir" -type f -name '*.rpm' | sort)
if [[ ${#rpm_files[@]} -gt 0 ]]; then
    if command -v rpm >/dev/null 2>&1; then
        for rpm_file in "${rpm_files[@]}"; do
            info "verifying $(basename "$rpm_file")"
            rpm -qlp "$rpm_file" | grep -q "/usr/bin/ply" || fail "$(basename "$rpm_file") does not contain /usr/bin/ply"
            rpm -qlp "$rpm_file" | grep -q "/usr/share/doc/ply/README.md" || fail "$(basename "$rpm_file") does not contain README.md"
            rpm -qlp "$rpm_file" | grep -q "/usr/share/doc/ply/CHANGELOG.md" || fail "$(basename "$rpm_file") does not contain CHANGELOG.md"
            rpm -qlp "$rpm_file" | grep -q "/usr/share/licenses/ply/LICENSE" || fail "$(basename "$rpm_file") does not contain LICENSE"
        done
    else
        info "rpm command not available; skipped .rpm content verification"
    fi
else
    info "no .rpm packages found; skipped .rpm verification"
fi

info "native Linux package verification passed"
