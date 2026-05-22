#!/usr/bin/env bash
set -euo pipefail

artifact_dir="target/distrib"
output_dir="target/distrib"
version=""
formats="deb,rpm"
target_filter=""
deb_arch_filter=""
rpm_arch_filter=""

usage() {
    cat >&2 <<'EOF'
usage: package-linux-native-artifacts.sh --version <version> [--artifact-dir <dir>] [--output-dir <dir>] [--formats deb,rpm] [--target <rust-target>] [--deb-arch <deb-arch>] [--rpm-arch <rpm-arch>]
EOF
    exit 1
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --version)
            [[ $# -ge 2 ]] || usage
            version="$2"
            shift 2
            ;;
        --artifact-dir)
            [[ $# -ge 2 ]] || usage
            artifact_dir="$2"
            shift 2
            ;;
        --output-dir)
            [[ $# -ge 2 ]] || usage
            output_dir="$2"
            shift 2
            ;;
        --formats)
            [[ $# -ge 2 ]] || usage
            formats="$2"
            shift 2
            ;;
        --target)
            [[ $# -ge 2 ]] || usage
            target_filter="$2"
            shift 2
            ;;
        --deb-arch)
            [[ $# -ge 2 ]] || usage
            deb_arch_filter="$2"
            shift 2
            ;;
        --rpm-arch)
            [[ $# -ge 2 ]] || usage
            rpm_arch_filter="$2"
            shift 2
            ;;
        *)
            usage
            ;;
    esac
done

[[ -n "$version" ]] || usage
[[ -d "$artifact_dir" ]] || {
    printf 'error: artifact directory not found: %s\n' "$artifact_dir" >&2
    exit 1
}

want_deb=false
want_rpm=false
IFS=',' read -r -a selected_formats <<<"$formats"
for format in "${selected_formats[@]}"; do
    case "$format" in
        deb) want_deb=true ;;
        rpm) want_rpm=true ;;
        *) printf 'error: unsupported format: %s\n' "$format" >&2; exit 1 ;;
    esac
done

if $want_deb && ! command -v dpkg-deb >/dev/null 2>&1; then
    printf 'error: dpkg-deb is required to build .deb packages\n' >&2
    exit 1
fi

if $want_rpm && ! command -v rpmbuild >/dev/null 2>&1; then
    printf 'error: rpmbuild is required to build .rpm packages\n' >&2
    exit 1
fi

mkdir -p "$output_dir"
work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT HUP INT TERM

upstream_version="${version#v}"
deb_version="$upstream_version"
deb_revision="1"
rpm_version="$upstream_version"
rpm_release="1"

if [[ "$upstream_version" == *-* ]]; then
    base_version="${upstream_version%%-*}"
    prerelease="${upstream_version#"$base_version"-}"
    deb_version="${base_version}~${prerelease}"
    rpm_version="$base_version"
    rpm_release="0.${prerelease//-/.}.1"
fi

description_short="Local-first package manager for coding-agent assets."
description_long="Ply installs reusable Codex, Claude Code, and related agent assets into existing repositories without taking ownership of repository-managed context."
maintainer_name="Jean Simeoni"
maintainer_email="opensource@users.noreply.github.com"
homepage="https://plycli.dev"
license="GPL-3.0-only"

package_archives=(
    "x86_64-unknown-linux-musl:amd64:x86_64"
    "aarch64-unknown-linux-musl:arm64:aarch64"
)

find_single_file() {
    local root="$1"
    local name="$2"
    local path
    path="$(find "$root" -type f -name "$name" -print -quit)"
    [[ -n "$path" ]] || {
        printf 'error: %s not found in %s\n' "$name" "$root" >&2
        exit 1
    }
    printf '%s' "$path"
}

build_deb() {
    local deb_arch="$1"
    local extracted_root="$2"
    local output_path="$3"
    local package_root="$work_dir/deb-$deb_arch"
    local control_dir="$package_root/DEBIAN"
    local binary license_file readme changelog

    rm -rf "$package_root"
    mkdir -p "$control_dir" "$package_root/usr/bin" "$package_root/usr/share/doc/ply" "$package_root/usr/share/licenses/ply"

    binary="$(find_single_file "$extracted_root" ply)"
    license_file="$(find_single_file "$extracted_root" LICENSE)"
    readme="$(find_single_file "$extracted_root" README.md)"
    changelog="$(find_single_file "$extracted_root" CHANGELOG.md)"

    install -Dm755 "$binary" "$package_root/usr/bin/ply"
    install -Dm644 "$license_file" "$package_root/usr/share/licenses/ply/LICENSE"
    install -Dm644 "$readme" "$package_root/usr/share/doc/ply/README.md"
    install -Dm644 "$changelog" "$package_root/usr/share/doc/ply/CHANGELOG.md"

    cat >"$control_dir/control" <<EOF
Package: ply
Version: ${deb_version}-${deb_revision}
Section: utils
Priority: optional
Architecture: $deb_arch
Maintainer: ${maintainer_name} <${maintainer_email}>
Homepage: $homepage
Description: $description_short
 $description_long
EOF

    dpkg-deb --build --root-owner-group "$package_root" "$output_path" >/dev/null
}

build_rpm() {
    local rpm_arch="$1"
    local extracted_root="$2"
    local output_dir_path="$3"
    local rpmbuild_root="$work_dir/rpmbuild-$rpm_arch"
    local build_root="$rpmbuild_root/BUILDROOT/ply-$rpm_arch"
    local spec_path="$rpmbuild_root/SPECS/ply.spec"
    local binary license_file readme changelog

    rm -rf "$rpmbuild_root"
    mkdir -p \
        "$rpmbuild_root/BUILD" \
        "$rpmbuild_root/BUILDROOT" \
        "$rpmbuild_root/RPMS" \
        "$rpmbuild_root/SOURCES" \
        "$rpmbuild_root/SPECS" \
        "$rpmbuild_root/SRPMS" \
        "$build_root/usr/bin" \
        "$build_root/usr/share/doc/ply" \
        "$build_root/usr/share/licenses/ply"

    binary="$(find_single_file "$extracted_root" ply)"
    license_file="$(find_single_file "$extracted_root" LICENSE)"
    readme="$(find_single_file "$extracted_root" README.md)"
    changelog="$(find_single_file "$extracted_root" CHANGELOG.md)"

    install -Dm755 "$binary" "$build_root/usr/bin/ply"
    install -Dm644 "$license_file" "$build_root/usr/share/licenses/ply/LICENSE"
    install -Dm644 "$readme" "$build_root/usr/share/doc/ply/README.md"
    install -Dm644 "$changelog" "$build_root/usr/share/doc/ply/CHANGELOG.md"

    cat >"$spec_path" <<EOF
Name: ply
Version: $rpm_version
Release: $rpm_release
Summary: $description_short
License: $license
URL: $homepage
BuildArch: $rpm_arch
AutoReqProv: no

%description
$description_long

%install
mkdir -p %{buildroot}
cp -a "$build_root"/. %{buildroot}/

%files
/usr/bin/ply
/usr/share/doc/ply/README.md
/usr/share/doc/ply/CHANGELOG.md
/usr/share/licenses/ply/LICENSE
EOF

    rpmbuild \
        --define "_topdir $rpmbuild_root" \
        --define "_rpmdir $output_dir_path" \
        --define "_build_id_links none" \
        -bb "$spec_path" >/dev/null
}

for package_def in "${package_archives[@]}"; do
    IFS=':' read -r rust_target deb_arch rpm_arch <<<"$package_def"

    if [[ -n "$target_filter" && "$rust_target" != "$target_filter" ]]; then
        continue
    fi
    if [[ -n "$deb_arch_filter" && "$deb_arch" != "$deb_arch_filter" ]]; then
        continue
    fi
    if [[ -n "$rpm_arch_filter" && "$rpm_arch" != "$rpm_arch_filter" ]]; then
        continue
    fi

    archive_path="$artifact_dir/ply-$rust_target.tar.xz"
    [[ -f "$archive_path" ]] || {
        printf 'error: required release archive not found: %s\n' "$archive_path" >&2
        exit 1
    }

    extract_dir="$work_dir/extract-$rust_target"
    mkdir -p "$extract_dir"
    tar -xJf "$archive_path" -C "$extract_dir"

    if $want_deb; then
        build_deb "$deb_arch" "$extract_dir" "$output_dir/ply_${deb_version}-${deb_revision}_${deb_arch}.deb"
    fi

    if $want_rpm; then
        build_rpm "$rpm_arch" "$extract_dir" "$output_dir"
    fi
done
