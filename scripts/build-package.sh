#!/usr/bin/env bash
# Builds the release archive: dist/gnome-gcal-<version>-<arch>.tar.gz
# Contents (single top-level directory):
#   gnome-gcal, uninstall.sh, data/, assets/, README.md, LICENSE, CHANGELOG.md
set -euo pipefail
cd "$(dirname "$0")/.."

version=$(./scripts/check-version.sh)
arch=${PACKAGE_ARCH:-$(uname -m)}
[[ $arch == arm64 ]] && arch=aarch64
case "$arch" in
    x86_64|aarch64) ;;
    *) echo "Unsupported architecture: $arch" >&2; exit 1 ;;
esac

package="gnome-gcal-${version}-${arch}"
stage="dist/${package}"

cargo build --release --locked

rm -rf -- "$stage" "dist/${package}.tar.gz"
install -d "$stage/data" "$stage/assets"
install -m 0755 target/release/gnome-gcal "$stage/gnome-gcal"
install -m 0755 uninstall.sh "$stage/uninstall.sh"
install -m 0644 data/*.desktop.in "$stage/data/"
install -m 0644 assets/*.svg "$stage/assets/"
for doc in README.md LICENSE CHANGELOG.md; do
    [[ -f "$doc" ]] && install -m 0644 "$doc" "$stage/$doc"
done

epoch=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct 2>/dev/null || date +%s)}
tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="@${epoch}" \
    -cf - -C dist "$package" | gzip -n -9 > "dist/${package}.tar.gz"
rm -rf -- "$stage"
echo "dist/${package}.tar.gz"
