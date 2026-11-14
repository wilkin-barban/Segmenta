#!/usr/bin/env bash
#
# Fetch the pinned yt-dlp zipapp that the .deb ships (S21).
#
#   1. reads `packaging/ytdlp.lock` (version + SHA-256, both tracked in git);
#   2. downloads https://github.com/yt-dlp/yt-dlp/releases/download/<version>/yt-dlp
#      to a scratch file — never straight to the final path, so a failed or
#      truncated transfer cannot leave a half-written interpreter behind;
#   3. refuses to install it unless the SHA-256 matches the lock file;
#   4. moves it to `dist/yt-dlp` with mode 755, ready for `cargo deb`.
#
# It is idempotent: when `dist/yt-dlp` already has the locked digest nothing is
# downloaded and no network is touched. No `sudo`, no writes outside `dist/`.
#
# Usage:
#   packaging/fetch-ytdlp.sh [output-dir]      # default: dist
#
# Environment:
#   YTDLP_LOCK   path to the lock file (default: packaging/ytdlp.lock)
#   CURL         curl binary (default: curl); wget is used when curl is absent
#
set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
readonly LOCK="${YTDLP_LOCK:-$SCRIPT_DIR/ytdlp.lock}"
readonly OUT_DIR="${1:-$REPO_ROOT/dist}"
readonly DEST="$OUT_DIR/yt-dlp"

die() {
    printf 'ERROR: %s\n' "$*" >&2
    exit 1
}

[ -f "$LOCK" ] || die "lock file not found at $LOCK"

lock_value() {
    # First `key=value` line; the value must not be empty.
    sed -n "s/^$1=\([^[:space:]]*\)$/\1/p" "$LOCK" | head -n 1
}

VERSION="$(lock_value version)"
EXPECTED="$(lock_value sha256)"

[ -n "$VERSION" ] || die "$LOCK does not declare version="
[ -n "$EXPECTED" ] || die "$LOCK does not declare sha256="
case "$EXPECTED" in
    *[!0-9a-f]* | ????????????????????????????????????????????????) die "sha256= in $LOCK is not a lowercase hex digest: '$EXPECTED'" ;;
    *) [ "${#EXPECTED}" -eq 64 ] || die "sha256= in $LOCK must be 64 hex characters (got ${#EXPECTED})" ;;
esac

URL="https://github.com/yt-dlp/yt-dlp/releases/download/$VERSION/yt-dlp"

digest_of() {
    # sha256sum is coreutils; shasum -a 256 is the macOS fallback.
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum -- "$1" | cut -d' ' -f1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 -- "$1" | cut -d' ' -f1
    else
        die "no sha256 tool available (install coreutils for sha256sum)"
    fi
}

# --- idempotence ------------------------------------------------------------
if [ -f "$DEST" ] && [ "$(digest_of "$DEST")" = "$EXPECTED" ]; then
    printf 'yt-dlp %s already at %s (sha256 verified)\n' "$VERSION" "$DEST"
    exit 0
fi

# --- download ---------------------------------------------------------------
mkdir -p -- "$OUT_DIR"
SCRATCH="$(mktemp "$OUT_DIR/.yt-dlp.XXXXXXXX")" || die "cannot create a scratch file in $OUT_DIR"
cleanup() {
    status=$?
    [ -f "$SCRATCH" ] && rm -f -- "$SCRATCH"
    exit "$status"
}
trap cleanup EXIT

printf 'fetching yt-dlp %s\n  %s\n' "$VERSION" "$URL"

if command -v "${CURL:-curl}" >/dev/null 2>&1; then
    "${CURL:-curl}" -fsSL --proto '=https' --tlsv1.2 --retry 3 --retry-delay 2 \
        -o "$SCRATCH" "$URL" ||
        die "download failed: curl could not fetch $URL (no network, or the release asset is gone)"
elif command -v wget >/dev/null 2>&1; then
    wget -q --https-only -O "$SCRATCH" "$URL" ||
        die "download failed: wget could not fetch $URL (no network, or the release asset is gone)"
else
    die "neither curl nor wget is available; install one of them"
fi

[ -s "$SCRATCH" ] || die "downloaded file is empty: $URL"

# --- verify -----------------------------------------------------------------
ACTUAL="$(digest_of "$SCRATCH")"
if [ "$ACTUAL" != "$EXPECTED" ]; then
    die "SHA-256 mismatch for $URL: expected $EXPECTED, got $ACTUAL. The lock file and the published asset disagree — do not ship this file."
fi

# A zipapp is a Python zip with a shebang; catch a truncated or HTML response
# that somehow matched nothing else.
head -c 2 -- "$SCRATCH" | grep -q '#!' ||
    die "$URL did not return the zipapp (no shebang); the release asset may have changed shape"

chmod 755 -- "$SCRATCH"
mv -f -- "$SCRATCH" "$DEST"
SCRATCH=""
printf 'wrote %s (%s bytes, yt-dlp %s, sha256 verified)\n' "$DEST" "$(wc -c <"$DEST")" "$VERSION"
