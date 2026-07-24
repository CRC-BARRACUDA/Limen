#!/usr/bin/env bash
# Build a release and package Linux distribution artifacts into ./dist.
#
# Produces, for the host architecture:
#   dist/Limen-<ver>-linux-<arch>.tar.gz          distribution tarball (GUI + CLI + LICENSE)
#   dist/Limen-<ver>-linux-<arch>.tar.gz.sha256   its checksum
set -euo pipefail

cd "$(dirname "$0")/.."
root=$(pwd)

arch=$(uname -m)                                   # e.g. x86_64
ver=$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')
stem="Limen-${ver}-linux-${arch}"

echo ">> building release (Limen + limen-cli)"
cargo build --release -p limen-gui -p limen-cli

dist="$root/dist"
rm -rf "$dist" "$dist/.stage"
mkdir -p "$dist"
stage="$dist/.stage/$stem"
mkdir -p "$stage"

echo ">> staging artifacts"
install -m755 target/release/Limen "$stage/Limen"
install -m755 target/release/limen-cli "$stage/limen-cli"
install -m644 LICENSE "$stage/LICENSE"

# Strip to shrink the download (keep the originals in target/ intact).
strip "$stage/Limen" "$stage/limen-cli" 2>/dev/null || true

echo ">> writing $stem.tar.gz"
tar -C "$dist/.stage" -czf "$dist/$stem.tar.gz" "$stem"

rm -rf "$dist/.stage"

echo ">> checksum"
( cd "$dist" && sha256sum "$stem.tar.gz" > "$stem.tar.gz.sha256" )

echo
echo "artifacts in dist/:"
ls -lh "$dist"
