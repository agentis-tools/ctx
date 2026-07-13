# Packaging and distribution

This document is the maintainer runbook for publishing `ctx`. The crates.io package is
`agentis-ctx`; every distribution installs the executable named `ctx` (`ctx.exe` on Windows).
The project is dual-licensed under MIT or Apache-2.0.

## Release assets

A `vX.Y.Z` release publishes these archives for the targets currently supported by CI:

```text
ctx-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz
ctx-vX.Y.Z-x86_64-apple-darwin.tar.gz
ctx-vX.Y.Z-aarch64-apple-darwin.tar.gz
ctx-vX.Y.Z-x86_64-pc-windows-msvc.zip
```

Archives contain the executable, `README.md`, `LICENSE-MIT`, and `LICENSE-APACHE`. Unix archives
use `ctx`; the Windows ZIP uses `ctx.exe`. `SHA256SUMS` contains the SHA-256 digest of every
release artifact, including `.deb` and `.rpm` packages. Generate checksums only from completed
artifacts; never place placeholder hashes in package definitions.

Linux ARM64 and musl are not published because the existing CI build has not established reliable
builds for those targets. Consequently, Homebrew and AUR definitions must not claim Linux ARM64,
and no `arm64.deb` or `aarch64.rpm` should be generated.

## Updating package definitions

After downloading a release's `SHA256SUMS`, run the repository generators with the unprefixed
version:

```bash
./scripts/update-homebrew-formula.sh 0.3.5 SHA256SUMS
./scripts/update-aur-package.sh 0.3.5 SHA256SUMS
pwsh ./scripts/update-scoop-manifest.ps1 -Version 0.3.5 -Checksums SHA256SUMS
```

The resulting files are ready to copy to the external package-index repositories:

- `packaging/homebrew/ctx.rb` → `agentis-tools/homebrew-tap/Formula/ctx.rb`
- `packaging/aur/PKGBUILD` and `.SRCINFO` → the `ctx-bin` AUR Git repository
- `packaging/scoop/ctx.json` → `agentis-tools/scoop-bucket/bucket/ctx.json`

The release workflow currently generates these definitions but does not publish to external
repositories. External publication is manual; no credentials belong in this repository. If
Homebrew publishing is later enabled, it must be opt-in and use a repository secret named
`HOMEBREW_TAP_TOKEN`. AUR and Scoop publication likewise require explicit repository credentials.
The existing crates.io release job uses `CARGO_REGISTRY_TOKEN`.

## Local validation

Validate the generated definitions on their native platforms where the tools are available:

```bash
brew audit --strict agentis-tools/tap/ctx
brew test agentis-tools/tap/ctx

cd packaging/aur
makepkg --printsrcinfo
makepkg --cleanbuild

dpkg-deb --info ctx_VERSION_amd64.deb
dpkg-deb --contents ctx_VERSION_amd64.deb
sudo apt install ./ctx_VERSION_amd64.deb
ctx --version
ctx --help

rpm -qip ctx-VERSION-1.x86_64.rpm
rpm -qlp ctx-VERSION-1.x86_64.rpm
sudo dnf install ./ctx-VERSION-1.x86_64.rpm
ctx --version
ctx --help
```

For Scoop, run the bucket manifest validator used by the Scoop project, then install from a local
test bucket and run `ctx --version` and `ctx --help`. Package installation tests must not access the
network, invoke `ctx self-update`, or modify package-owned files outside the package transaction.

Before tagging, also run the repository checks:

```bash
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets
```

## Upgrade ownership

Only a binary downloaded directly from GitHub Releases should use `ctx self-update`. Managed
installations must be upgraded using their owner:

```text
Cargo       cargo install agentis-ctx
Homebrew    brew upgrade ctx
Arch/AUR    yay -Syu ctx-bin
Scoop       scoop update ctx
Debian      install the newer .deb with apt
RPM         install the newer .rpm with dnf
```

`ctx self-update` recognizes Cargo, Homebrew, and Scoop installation layouts. On Linux it also asks
the local `pacman`, `dpkg`, and RPM databases whether they own the running executable. Detection
happens before network access or filesystem mutation, and ctx refuses to replace a managed file.
