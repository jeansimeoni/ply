# `ply-bin` AUR Packaging

This directory contains the in-repo source-of-truth for the `ply-bin`
package published to the Arch User Repository.

`ply-bin` installs the prebuilt Linux musl archives from GitHub Releases.
Its package homepage should point to <https://plycli.dev>. It does not build
from source.

## Files

- `PKGBUILD`: committed baseline package metadata for the current stable release
- `.SRCINFO`: committed generated metadata for the current stable release
- `LICENSE`: 0BSD license for the AUR repository source files
- `testdata/sha256.sum`: deterministic fixture for local and CI generation checks
- `scripts/generate-aur-ply-bin.sh`: renders `PKGBUILD` and `.SRCINFO`
- `scripts/check-aur-packaging.sh`: verifies the generator output in normal CI
- `.github/workflows/aur.yml`: validates generated packaging and pushes updates to
  the AUR repository on stable releases

## Manual bootstrap

1. Verify `ply-bin` is still available in the AUR and that your local AUR SSH
   key can authenticate to `aur.archlinux.org`.
2. Generate the package metadata for the first stable release:

```bash
tmp_dir="$(mktemp -d)"
release_tag="vX.Y.Z"
release_version="${release_tag#v}"
curl -LsSf "https://github.com/jeansimeoni/ply/releases/download/${release_tag}/sha256.sum" -o "$tmp_dir/sha256.sum"
scripts/generate-aur-ply-bin.sh \
  --version "$release_version" \
  --sha256-file "$tmp_dir/sha256.sum" \
  --maintainer-name "Jean Simeoni" \
  --maintainer-email "opensource@users.noreply.github.com" \
  --output-dir "$tmp_dir"
```

3. Clone the AUR package repo:

```bash
git -c init.defaultBranch=master clone ssh://aur@aur.archlinux.org/ply-bin.git
```

For a new package, Git should warn that the repository is empty. If it is not
empty, review the existing history before pushing.

4. Add the initial repository contents and make sure the local branch is
   `master`:

```bash
cd ply-bin
git branch -M master
install -Dm644 "$tmp_dir/PKGBUILD" PKGBUILD
install -Dm644 "$tmp_dir/.SRCINFO" .SRCINFO
install -Dm644 /home/jeansimeoni/Projects/ply/packaging/aur/ply-bin/LICENSE LICENSE
```

5. Optionally set a repo-local Git identity for AUR commits, then create the
   first commit and push:

```bash
git config user.name "Jean Simeoni"
git config user.email "opensource@users.noreply.github.com"
git add PKGBUILD .SRCINFO LICENSE
git commit -m "Add initial ply-bin package"
git push origin master
```

After that first push, the `AUR` GitHub workflow can update the package
automatically for later stable releases.
