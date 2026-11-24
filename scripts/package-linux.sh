#!/usr/bin/env bash
# Build a release and package Linux distribution artifacts into ./dist.
#
# Produces, for the host architecture:
#   dist/Limen-<ver>-linux-<arch>.tar.gz          distribution tarball (GUI + CLI + LICENSE)
#   dist/Limen-<ver>-linux-<arch>.tar.gz.sha256   its checksum
#   dist/Limen-<ver>-linux-<arch>.tar.gz.sig      its signature, by the team's key
#
# The signature is what Limen's updater checks before installing (see
# src/limen-sign). It is made with gpg and the team's key, which has to be in
# this machine's keyring; gpg asks for its passphrase. Without the key the
# build is left unsigned, and says so: an unsigned build installs only after a
# warning. LIMEN_SIGN_KEY names another key; LIMEN_SIGN=0 skips signing.
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
# JetBrains Mono is embedded in the GUI binary; the OFL requires its licence to
# travel with any redistribution.
install -m644 resources/fonts/JetBrainsMono-OFL.txt "$stage/JetBrainsMono-OFL.txt"

# The CLI as built, to check the signature with below (the staged copy goes).
stage_cli="$root/target/release/limen-cli"

# Strip to shrink the download (keep the originals in target/ intact).
strip "$stage/Limen" "$stage/limen-cli" 2>/dev/null || true

echo ">> writing $stem.tar.gz"
tar -C "$dist/.stage" -czf "$dist/$stem.tar.gz" "$stem"

rm -rf "$dist/.stage"

echo ">> checksum"
( cd "$dist" && sha256sum "$stem.tar.gz" > "$stem.tar.gz.sha256" )

# The team's key (CRC-BARRACUDA), by fingerprint.
sign_key="${LIMEN_SIGN_KEY:-8D151FD231907502BBFA528E140ABA21244518B8}"
if [ "${LIMEN_SIGN:-1}" = "0" ]; then
    echo ">> NOT SIGNED (LIMEN_SIGN=0)"
elif ! command -v gpg >/dev/null 2>&1 || ! gpg --list-secret-keys "$sign_key" >/dev/null 2>&1; then
    echo ">> NOT SIGNED: the signing key $sign_key is not in this machine's keyring."
    echo "   Limen will warn before installing this build. Do not publish it as a release."
else
    echo ">> signing with $sign_key"
    gpg --batch --yes --local-user "$sign_key" --detach-sign -o "$dist/$stem.tar.gz.sig" "$dist/$stem.tar.gz"
    # Checked the way Limen will check it: by the keys built into this build.
    quiet=/dev/stderr; [ -z "${LIMEN_SIGN_KEY:-}" ] || quiet=/dev/null
    if "$stage_cli" verify-signature "$dist/$stem.tar.gz" "$dist/$stem.tar.gz.sig" 2>"$quiet"; then
        :
    elif [ -n "${LIMEN_SIGN_KEY:-}" ]; then
        echo "   (signed with LIMEN_SIGN_KEY, which is not one of the keys built into Limen)"
    else
        echo ">> the signature does not verify against the keys built into Limen" >&2
        exit 1
    fi
fi

echo
echo "artifacts in dist/:"
ls -lh "$dist"
