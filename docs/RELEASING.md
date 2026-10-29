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

## 4. Update the Omarchy package

`omadeck-bin` in [omacom/omarchy-pkgs](https://github.com/omacom/omarchy-pkgs) repackages the
release tarball. Its `.omarchy/package.json` watches this repo's GitHub releases, and its
`package()` copies files from fixed paths in the tarball. So every release must keep:

- the tag format `vX.Y.Z`, and the asset name `omadeck-X.Y.Z-x86_64-linux.tar.gz`;
- `SHA256SUMS` beside it;
- the tarball's layout (`bin/`, `packaging/`, `assets/`, `examples/`, `LICENSE` under
  `omadeck-X.Y.Z-x86_64-linux/`).

Before tagging, check the layout against the previous release:

```bash
gh release download vPREVIOUS -p '*.tar.gz' -D /tmp/omadeck-prev
scripts/package.sh
diff <(tar tzf /tmp/omadeck-prev/*.tar.gz | sed 's|^[^/]*/||') \
     <(tar tzf dist/omadeck-*.tar.gz | sed 's|^[^/]*/||')   # prints nothing
```

If a release needs a new system package at runtime, add it to the `depends` of the Omarchy
package (and of `packaging/aur/*`) in the same update, and say so in the PR.

**Once the package is in omarchy-pkgs:** nothing to do. Their bot sees the new release and
opens the update PR itself.

**While its PR is still open:** refresh it once per release, as one small commit on top. In
the omarchy-pkgs clone, `origin` is omacom and `fork` is ours. Test with current master's
tooling, not the branch's:

```bash
cd ~/Work/omarchy-pkgs
git checkout add-omadeck-bin && git pull --ff-only fork add-omadeck-bin
git fetch origin master
git worktree add ../omarchy-pkgs-test origin/master
rm -rf ../omarchy-pkgs-test/pkgbuilds/omadeck-bin
cp -r pkgbuilds/omadeck-bin ../omarchy-pkgs-test/pkgbuilds/
cd ../omarchy-pkgs-test
python helpers/upstream-watch.py sync pkgbuilds/omadeck-bin    # moves pkgver and the checksum
python helpers/upstream-watch.py check pkgbuilds/omadeck-bin   # "already current"
tests/pinned-sources.sh && tests/package-metadata.sh            # PASS
bin/build --package omadeck-bin                                # Built: 1, Failed: 0 (needs Docker)
cp pkgbuilds/omadeck-bin/PKGBUILD ../omarchy-pkgs/pkgbuilds/omadeck-bin/
cd ../omarchy-pkgs && git worktree remove --force ../omarchy-pkgs-test
git diff                                                       # only pkgver and sha256sums
git commit -am "omadeck-bin: update to X.Y.Z"
git push fork add-omadeck-bin                                  # a normal push
```

Then update the version in the PR description, and post one short comment ("Updated to
omadeck X.Y.Z: …, built with `bin/build` against current master"), with no @-mentions.
Never force-push, rebase, or click "Update branch": one commit per release keeps the history
easy to review.
