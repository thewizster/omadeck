# Releasing omadeck

## 1. Prepare

1. Bump `version` in `Cargo.toml` (`[workspace.package]`), then run `cargo build` so `Cargo.lock` updates.
2. In `CHANGELOG.md`, rename **Unreleased** to `## [X.Y.Z] - YYYY-MM-DD`, add a fresh empty
   **Unreleased** section, and update the compare links at the bottom.
3. Update `pkgver` in `packaging/aur/omadeck/PKGBUILD` (reset `pkgrel=1`), then regenerate:
   `cd packaging/aur/omadeck && makepkg --printsrcinfo > .SRCINFO`
4. Update the version in the install commands in `README.md` and `site/index.html`
   (`grep -n 'releases/download' README.md site/index.html`).
5. Commit: `git commit -am "Release vX.Y.Z"`.

## 2. Tag and push

```bash
git tag -a vX.Y.Z -m "omadeck vX.Y.Z"
git push origin master vX.Y.Z
```

The **Release** workflow tests the build, packages `omadeck-X.Y.Z-x86_64-linux.tar.gz`, and publishes a
GitHub release with:

- the tarball and `SHA256SUMS`;
- `PKGBUILD-omadeck-bin`, with the tarball's checksum already filled in;
- release notes taken from this version's `CHANGELOG.md` section.

The tag must match the Cargo version, or the workflow stops.

## 3. Update the AUR

The AUR packages live in their own git repos. One-time setup: create an AUR account, add your SSH key,
then clone both repos (cloning a name that doesn't exist yet creates it on first push):

```bash
git clone ssh://aur@aur.archlinux.org/omadeck.git     aur-omadeck
git clone ssh://aur@aur.archlinux.org/omadeck-bin.git aur-omadeck-bin
```

Each release:

```bash
# Source package
cp packaging/aur/omadeck/{PKGBUILD,omadeck.install,.SRCINFO} aur-omadeck/
(cd aur-omadeck && makepkg -f && git add -A && git commit -m "vX.Y.Z" && git push)

# Prebuilt package: use the PKGBUILD attached to the GitHub release
gh release download vX.Y.Z -p PKGBUILD-omadeck-bin -O aur-omadeck-bin/PKGBUILD
cp packaging/aur/omadeck-bin/omadeck.install aur-omadeck-bin/
(cd aur-omadeck-bin && makepkg -f && makepkg --printsrcinfo > .SRCINFO \
  && git add -A && git commit -m "vX.Y.Z" && git push)
```

Running `makepkg -f` before each push makes sure the package really builds.
