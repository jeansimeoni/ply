# Install

Ply `0.1.0-rc2` is available through GitHub Releases, the shell installer,
downloadable native Linux packages, and source builds.

Project website: <https://plycli.dev>

## Requirements

- macOS or Linux
- `git`
- a shell environment with a writable `PATH` destination

For source builds, you also need:

- `mise` or a compatible Rust toolchain setup
- the Rust version pinned by `rust-toolchain.toml` and `mise.toml`

## GitHub Releases

All stable releases are published at:

<https://github.com/jeansimeoni/ply/releases>

The `v0.1.0-rc2` release includes:

- macOS archives for `x86_64` and `aarch64`
- Linux musl archives for `x86_64` and `aarch64`
- `sha256.sum`
- `ply-installer.sh`
- source tarball

If you prefer a manual archive install, download the asset for your platform,
extract it, and place `ply` somewhere on your `PATH`.

## Shell Installer

The shell installer downloads the matching release artifact and installs
`ply` into `${CARGO_HOME:-$HOME/.cargo}/bin`.

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/jeansimeoni/ply/releases/download/v0.1.0-rc2/ply-installer.sh | sh
```

Confirm the installed binary:

```bash
ply -V
```

## Homebrew

Stable releases are published to the maintainer tap:

```bash
brew install jeansimeoni/tap/ply
```

Upgrade later with:

```bash
brew upgrade ply
```

Pre-release tags do not publish to Homebrew.

## Native Linux Packages

Downloadable Linux packages are attached to each GitHub Release:

- `.deb` for Debian and Ubuntu style systems
- `.rpm` for Fedora and compatible systems

These are direct download artifacts, not apt or dnf repositories.

Install a downloaded `.deb`:

```bash
sudo dpkg -i ply_0.1.0~rc2-1_amd64.deb
```

Install a downloaded `.rpm`:

```bash
sudo dnf install ./ply-0.1.0-0.rc2.1.x86_64.rpm
```

## AUR And yay

Stable releases are published to the AUR package:

```bash
yay -S ply-bin
```

Pre-release tags do not publish to AUR. The stable `ply-bin` package installs
the same Linux musl release archives published on GitHub Releases.

## Build From Source

Clone the repository and build an optimized binary:

```bash
git clone https://github.com/jeansimeoni/ply.git
cd ply
mise install
mise exec -- cargo build --release
```

Run the binary directly:

```bash
./target/release/ply
```

Check the version:

```bash
./target/release/ply -V
```

## Manual Local Install

After building from source, copy the binary into a directory on your `PATH`:

```bash
install -Dm755 target/release/ply ~/.local/bin/ply
```

Confirm that your shell finds the installed binary:

```bash
ply -V
```

If `~/.local/bin` is not on your `PATH`, add it in your shell configuration.

Release archives include checksums. Verify downloaded archives against
`sha256.sum` from the GitHub Release.

## Update

If you installed with the shell installer, rerun the installer for the newer
release version.

For a source checkout:

```bash
git pull
mise install
mise exec -- cargo build --release
```

If you manually installed the binary, copy the rebuilt binary again:

```bash
install -Dm755 target/release/ply ~/.local/bin/ply
```

For Homebrew installs:

```bash
brew upgrade ply
```

For AUR installs:

```bash
yay -Syu ply-bin
```

## Uninstall

For a manual local install, remove the binary:

```bash
rm -f ~/.local/bin/ply
```

To remove the source checkout, delete the cloned repository directory.

For Homebrew, uninstall with:

```bash
brew uninstall ply
```

For AUR and yay, uninstall with:

```bash
yay -R ply-bin
```

For `.deb` installs:

```bash
sudo dpkg -r ply
```

For `.rpm` installs:

```bash
sudo dnf remove ply
```
