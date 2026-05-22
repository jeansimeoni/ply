#!/usr/bin/env sh
set -eu

usage() {
    printf 'usage: %s --version <version> --sha256-file <path> --output-dir <path>\n' "$0" >&2
    exit 1
}

version=''
sha256_file=''
output_dir=''

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version)
            [ "$#" -ge 2 ] || usage
            version="$2"
            shift 2
            ;;
        --sha256-file)
            [ "$#" -ge 2 ] || usage
            sha256_file="$2"
            shift 2
            ;;
        --output-dir)
            [ "$#" -ge 2 ] || usage
            output_dir="$2"
            shift 2
            ;;
        *)
            usage
            ;;
    esac
done

[ -n "$version" ] || usage
[ -n "$sha256_file" ] || usage
[ -n "$output_dir" ] || usage
[ -f "$sha256_file" ] || {
    printf 'error: sha256 file not found: %s\n' "$sha256_file" >&2
    exit 1
}

tag="$version"
pkgver="$version"
case "$version" in
    v*)
        pkgver="${version#v}"
        ;;
    *)
        tag="v$version"
        ;;
esac

release_base="https://github.com/jeansimeoni/ply/releases/download/$tag"
x86_64_asset='ply-x86_64-unknown-linux-musl.tar.xz'
aarch64_asset='ply-aarch64-unknown-linux-musl.tar.xz'

checksum_for() {
    asset_name="$1"
    checksum="$(
        awk -v asset="$asset_name" '
            {
                name = $NF
                sub(/^\*/, "", name)
                if (name == asset) {
                    print $1
                    exit
                }
            }
        ' "$sha256_file"
    )"
    [ -n "$checksum" ] || {
        printf 'error: checksum not found for %s in %s\n' "$asset_name" "$sha256_file" >&2
        exit 1
    }
    printf '%s' "$checksum"
}

x86_64_sha256="$(checksum_for "$x86_64_asset")"
aarch64_sha256="$(checksum_for "$aarch64_asset")"

mkdir -p "$output_dir"

pkgbuild_path="$output_dir/PKGBUILD"
srcinfo_path="$output_dir/.SRCINFO"

cat >"$pkgbuild_path" <<EOF
pkgname=ply-bin
pkgver=$pkgver
pkgrel=1
pkgdesc='Local-first package manager for coding-agent assets.'
arch=('x86_64' 'aarch64')
url='https://plycli.dev'
license=('GPL-3.0-only')
depends=('git')
provides=('ply')
conflicts=('ply')
source_x86_64=("ply-\${pkgver}-x86_64.tar.xz::$release_base/$x86_64_asset")
source_aarch64=("ply-\${pkgver}-aarch64.tar.xz::$release_base/$aarch64_asset")
sha256sums_x86_64=('$x86_64_sha256')
sha256sums_aarch64=('$aarch64_sha256')

package() {
    local archive=''
    local extract_dir="\${srcdir}/ply-\${pkgver}-pkg"

    case "\${CARCH}" in
        x86_64)
            archive="\${srcdir}/ply-\${pkgver}-x86_64.tar.xz"
            ;;
        aarch64)
            archive="\${srcdir}/ply-\${pkgver}-aarch64.tar.xz"
            ;;
        *)
            printf 'unsupported architecture: %s\n' "\${CARCH}" >&2
            return 1
            ;;
    esac

    rm -rf "\${extract_dir}"
    mkdir -p "\${extract_dir}"
    bsdtar -xf "\${archive}" -C "\${extract_dir}"

    install -Dm755 "\$(find "\${extract_dir}" -type f -name ply -print -quit)" "\${pkgdir}/usr/bin/ply"
    install -Dm644 "\$(find "\${extract_dir}" -type f -name LICENSE -print -quit)" "\${pkgdir}/usr/share/licenses/\${pkgname}/LICENSE"
    install -Dm644 "\$(find "\${extract_dir}" -type f -name README.md -print -quit)" "\${pkgdir}/usr/share/doc/\${pkgname}/README.md"
    install -Dm644 "\$(find "\${extract_dir}" -type f -name CHANGELOG.md -print -quit)" "\${pkgdir}/usr/share/doc/\${pkgname}/CHANGELOG.md"
}
EOF

cat >"$srcinfo_path" <<EOF
pkgbase = ply-bin
	pkgdesc = Local-first package manager for coding-agent assets.
	pkgver = $pkgver
	pkgrel = 1
	url = https://plycli.dev
	arch = x86_64
	arch = aarch64
	license = GPL-3.0-only
	depends = git
	provides = ply
	conflicts = ply
	source_x86_64 = ply-$pkgver-x86_64.tar.xz::$release_base/$x86_64_asset
	sha256sums_x86_64 = $x86_64_sha256
	source_aarch64 = ply-$pkgver-aarch64.tar.xz::$release_base/$aarch64_asset
	sha256sums_aarch64 = $aarch64_sha256

pkgname = ply-bin
EOF

printf 'generated %s and %s\n' "$pkgbuild_path" "$srcinfo_path"
