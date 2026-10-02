#!/usr/bin/env bash
# Build a release tarball in dist/:
#   omadeck-<version>-<arch>-linux.tar.gz   prebuilt binaries + install script
#   SHA256SUMS
#   PKGBUILD-omadeck-bin                    ready to copy into the AUR repo
#   RELEASE_NOTES.md                        this version's CHANGELOG section
# Used by .github/workflows/release.yml; also runs locally.
set -euo pipefail
cd "$(dirname "$0")/.."

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
arch=$(uname -m)
name="omadeck-$version-$arch-linux"
rm -rf dist && mkdir -p "dist/$name"

if [[ -z ${SKIP_BUILD:-} ]]; then
  cargo build --release --locked
fi

stage="dist/$name"
install -Dm755 target/release/omadeck "$stage/bin/omadeck"
install -Dm755 target/release/omadeckd "$stage/bin/omadeckd"
install -Dm755 scripts/install.sh "$stage/scripts/install.sh"
install -Dm755 scripts/uninstall.sh "$stage/scripts/uninstall.sh"
for f in packaging/70-omadeck.rules packaging/dev.omadeck.Omadeck.desktop packaging/omadeckd.service \
  assets/dev.omadeck.Omadeck.svg examples/config.toml LICENSE README.md CHANGELOG.md; do
  install -Dm644 "$f" "$stage/$f"
done

tar -C dist --owner=0 --group=0 --sort=name -czf "dist/$name.tar.gz" "$name"
rm -rf "$stage"
(cd dist && sha256sum "$name.tar.gz" >SHA256SUMS)
sha=$(cut -d' ' -f1 "dist/SHA256SUMS")

sed -e "s/@VERSION@/$version/" -e "s/@SHA256_X86_64@/$sha/" \
  packaging/aur/omadeck-bin/PKGBUILD.in >dist/PKGBUILD-omadeck-bin

# This version's section of the changelog becomes the release notes.
awk -v v="$version" '
  $0 ~ "^## \\[" v "\\]" { on = 1; next }
  on && /^## \[/ { exit }
  on { print }
' CHANGELOG.md >dist/RELEASE_NOTES.md

echo "dist/:"
ls -l dist
