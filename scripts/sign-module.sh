#!/usr/bin/env bash
# Sign a release of a module with the team's key, for Limen to check on install.
#
#   scripts/sign-module.sh <module-repo> [--ref REF] [--out DIR] [FILE...]
#
#   <module-repo>  the module's git repository (a working copy of it)
#   --ref REF      the commit that is released (default: HEAD) — the one the
#                  release's tag points at, and the tip of the default branch
#   --out DIR      where the signatures go (default: <module-repo>/dist)
#   FILE...        the release's other files to sign as they are: a native
#                  module's library or archive for each platform
#
# Writes:
#   limen-module.asc   the signature of the module's statement — one line:
#                      its name, version and the digest of its files
#   <FILE>.sig         one per FILE
#
# Upload them to the module's GitHub release for that version, beside its other
# files:   gh release upload v<version> <out>/limen-module.asc <out>/*.sig
#
# The statement is worked out from an export of REF — the files exactly as the
# repository has them, which is what Limen downloads — never from the working
# copy, which has files of its own (a config.json, build output).
#
# Signing again later — with a new key, say — is running this again and
# uploading over the old signatures: the module itself is not touched.
#
# The key has to be in this machine's keyring; gpg asks for its passphrase.
# LIMEN_SIGN_KEY names another key than the team's.
set -euo pipefail

limen_root=$(cd "$(dirname "$0")/.." && pwd)
sign_key="${LIMEN_SIGN_KEY:-8D151FD231907502BBFA528E140ABA21244518B8}"

repo="" ref="HEAD" out="" files=()
while [ $# -gt 0 ]; do
    case "$1" in
        --ref) ref="$2"; shift 2 ;;
        --out) out="$2"; shift 2 ;;
        -h|--help) sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit 0 ;;
        *) if [ -z "$repo" ]; then repo="$1"; else files+=("$1"); fi; shift ;;
    esac
done
[ -n "$repo" ] || { echo "usage: $0 <module-repo> [--ref REF] [--out DIR] [FILE...]" >&2; exit 2; }
repo=$(cd "$repo" && pwd)
git -C "$repo" rev-parse --git-dir >/dev/null 2>&1 || { echo "$repo is not a git repository" >&2; exit 2; }
commit=$(git -C "$repo" rev-parse --verify "$ref^{commit}")
out="${out:-$repo/dist}"
mkdir -p "$out"

# limen-cli works the statement out, exactly as Limen will on install.
# The newer of the release and debug builds: an old one may not know how.
cli="$limen_root/target/release/limen-cli"
debug="$limen_root/target/debug/limen-cli"
if [ ! -x "$cli" ] || { [ -x "$debug" ] && [ "$debug" -nt "$cli" ]; }; then cli="$debug"; fi
[ -x "$cli" ] && "$cli" module-statement --help >/dev/null 2>&1 || {
    echo "build limen-cli first: cargo build -p limen-cli" >&2; exit 2; }

gpg --list-secret-keys "$sign_key" >/dev/null 2>&1 || {
    echo "the signing key $sign_key is not in this machine's keyring" >&2; exit 2; }

export_dir=$(mktemp -d)
trap 'rm -rf "$export_dir"' EXIT
# As GitHub's own archive of the commit: no line endings rewritten on the way.
git -C "$repo" -c core.autocrlf=false archive "$commit" | tar -x -C "$export_dir"

"$cli" module-statement "$export_dir" > "$export_dir.statement"
trap 'rm -rf "$export_dir" "$export_dir.statement"' EXIT
echo ">> $(cat "$export_dir.statement")"
echo "   of ${commit:0:12} ($ref)"
if [ "$commit" != "$(git -C "$repo" rev-parse HEAD)" ] || [ -n "$(git -C "$repo" status --porcelain --untracked-files=no)" ]; then
    echo "   note: this is the commit's content, not the working copy's"
fi

echo ">> signing with $sign_key"
gpg --batch --yes --local-user "$sign_key" --armor --detach-sign -o "$out/limen-module.asc" "$export_dir.statement"
written=("$out/limen-module.asc")
for f in "${files[@]}"; do
    [ -f "$f" ] || { echo "no such file: $f" >&2; exit 2; }
    gpg --batch --yes --local-user "$sign_key" --detach-sign -o "$out/$(basename "$f").sig" "$f"
    written+=("$out/$(basename "$f").sig")
done

# Checked the way Limen will check them: by the keys built into this build.
ok=1
# (A key of one's own is not among them: that it is not is said once, below.)
quiet=/dev/stderr; [ -z "${LIMEN_SIGN_KEY:-}" ] || quiet=/dev/null
"$cli" verify-signature "$export_dir.statement" "$out/limen-module.asc" 2>"$quiet" || ok=0
for f in "${files[@]}"; do
    "$cli" verify-signature "$f" "$out/$(basename "$f").sig" 2>"$quiet" || ok=0
done
if [ "$ok" = 0 ]; then
    if [ -n "${LIMEN_SIGN_KEY:-}" ]; then
        echo "   (signed with LIMEN_SIGN_KEY, which is not one of the keys built into Limen)"
    else
        echo ">> a signature does not verify against the keys built into Limen" >&2
        exit 1
    fi
fi

version=$(awk '{print $3}' "$export_dir.statement")
echo
echo "written:"
printf '  %s\n' "${written[@]}"
echo "upload them to the release of this version, for example:"
echo "  gh release upload v$version ${written[*]}"
