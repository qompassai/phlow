#!/usr/bin/env bash
# Assemble the Linux release tarball: release build of the `phlow` binary
# plus the systemd units, desktop file, icon, and README from packaging/.
# Output: dist/phlow-<version>-x86_64-unknown-linux-gnu.tar.gz
#
#   scripts/publish/package-linux.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

VERSION=$(grep -m1 '^version' crates/phlow-cli/Cargo.toml | cut -d'"' -f2)
TARGET="x86_64-unknown-linux-gnu"
DIST="dist/phlow-$VERSION-$TARGET"
rm -rf "$DIST"
mkdir -p "$DIST"

echo "== cargo build --release -p phlow-cli =="
cargo build --release -p phlow-cli

install -m755 "target/release/phlow" "$DIST/phlow"
cp packaging/phlow.socket 'packaging/phlow@.service' \
   packaging/phlow-system.socket 'packaging/phlow-system@.service' \
   packaging/phlow.desktop "$DIST/"
mkdir -p "$DIST/icons"
cp packaging/icons/phlow.svg "$DIST/icons/"
cp README.md packaging/README.md "$DIST/"

tar -C dist -czf "dist/phlow-$VERSION-$TARGET.tar.gz" "phlow-$VERSION-$TARGET"
sha256sum "dist/phlow-$VERSION-$TARGET.tar.gz" | tee "dist/phlow-$VERSION-$TARGET.tar.gz.sha256"
echo "== wrote dist/phlow-$VERSION-$TARGET.tar.gz =="
