#!/usr/bin/env bash
#
# Segmenta packaging gate (OpenSpec 10, S13).
#
# Validates one generated .deb without installing it:
#   1. the file exists and is a .deb;
#   2. the control file declares the expected package, version, dependencies
#      (libsecret-tools included) and a Description;
#   3. every binary, systemd user unit, desktop entry, AppStream file, icon and
#      the license land in the documented installation tree;
#   4. desktop-file-validate and appstreamcli validate the *installed-tree copies*
#      extracted from the package itself, not the sources;
#   5. lintian runs when available (informational only: lintian findings are
#      reported for the reviewer, they never flip the gate by themselves).
#
# Usage:
#   packaging/check-package.sh dist/segmenta_0.0.2-1_amd64.deb [expected-version]
#
# The expected version defaults to 0.0.2 and can also be set with
# SEGMENTA_EXPECTED_VERSION. Exits non-zero on the first hard failure and always
# prints a one-line summary.
#
set -euo pipefail

readonly EXPECTED_PACKAGE="segmenta"
readonly EXPECTED_VERSION="${2:-${SEGMENTA_EXPECTED_VERSION:-0.0.2}}"

readonly REQUIRED_PATHS=(
    "usr/bin/segmenta"
    "usr/bin/segmenta-cli"
    "usr/bin/segmenta-daemon"
    "usr/bin/segmenta-gui"
    "usr/lib/systemd/user/segmenta.service"
    "usr/lib/systemd/user/segmenta.socket"
    "usr/share/applications/io.github.wilkin.Segmenta.desktop"
    "usr/share/metainfo/io.github.wilkin.Segmenta.metainfo.xml"
    "usr/share/icons/hicolor/scalable/apps/io.github.wilkin.Segmenta.svg"
    "usr/share/doc/segmenta/copyright"
)

DEB="${1:-}"
SCRATCH=""
PACKAGE_VERSION=""
LINTIAN_STATUS="not run"
used_paths=0

# Only the scratch directory created by mktemp -d is ever removed; no fixed path
# such as /tmp/... is touched. The original exit status is preserved.
cleanup() {
    status=$?
    if [ -n "$SCRATCH" ] && [ -d "$SCRATCH" ]; then
        rm -rf -- "$SCRATCH"
    fi
    exit "$status"
}
trap cleanup EXIT

die() {
    printf 'ERROR: %s\n' "$*" >&2
    printf 'summary: FAIL %s package=%s reason=%s\n' \
        "$(basename -- "${DEB:-<none>}")" "$EXPECTED_PACKAGE" "$*" >&2
    exit 1
}

summary_ok() {
    printf 'summary: OK %s package=%s version=%s paths=%d/%d lintian=%s\n' \
        "$(basename -- "$DEB")" "$EXPECTED_PACKAGE" "$PACKAGE_VERSION" \
        "$used_paths" "${#REQUIRED_PATHS[@]}" "$LINTIAN_STATUS"
}

# --- 1. input ---------------------------------------------------------------
[ -n "$DEB" ] || die "no .deb given (usage: packaging/check-package.sh <file.deb> [expected-version])"
[ -f "$DEB" ] || die "file not found"
case "$DEB" in
    *.deb) ;;
    *) die "not a .deb (expected a *.deb file)" ;;
esac
command -v dpkg-deb >/dev/null 2>&1 || die "dpkg-deb is not installed (install dpkg)"
printf 'checking %s\n' "$DEB"

# --- 2. control file --------------------------------------------------------
info="$(dpkg-deb --info "$DEB")" || die "dpkg-deb --info failed (not a Debian package?)"

# dpkg-deb --info indents the control fields by one space, so the patterns are
# tolerant about the leading whitespace.
printf '%s\n' "$info" | grep -Eq "^[[:space:]]*Package:[[:space:]]*$EXPECTED_PACKAGE[[:space:]]*$" ||
    die "control does not declare Package: $EXPECTED_PACKAGE"

PACKAGE_VERSION="$(dpkg-deb -f "$DEB" Version)" || die "dpkg-deb -f Version failed"
case "$PACKAGE_VERSION" in
    "$EXPECTED_VERSION" | "$EXPECTED_VERSION"-*) ;;
    *) die "unexpected Version '$PACKAGE_VERSION' (expected $EXPECTED_VERSION)" ;;
esac
printf '  version: %s\n' "$PACKAGE_VERSION"

depends="$(dpkg-deb -f "$DEB" Depends)" || die "dpkg-deb -f Depends failed"
depends_compact="$(printf '%s' "$depends" | tr -d '[:space:]')"
case ",$depends_compact," in
    *",libsecret-tools,"*) printf '  depends: %s\n' "$depends" ;;
    *) die "Depends does not include libsecret-tools (got: ${depends:-<empty>})" ;;
esac

printf '%s\n' "$info" | grep -Eq '^[[:space:]]*Description:[[:space:]]*[^[:space:]]' ||
    die "control has no Description field"

# --- 3. contents ------------------------------------------------------------
contents="$(dpkg-deb --contents "$DEB")" || die "dpkg-deb --contents failed"

missing=()
for path in "${REQUIRED_PATHS[@]}"; do
    if printf '%s\n' "$contents" |
        awk -v want="$path" '{ p = $6; sub(/^\.\//, "", p); if (p == want) found = 1 } END { exit found ? 0 : 1 }'; then
        used_paths=$((used_paths + 1))
    else
        missing+=("$path")
    fi
done

if [ "${#missing[@]}" -ne 0 ]; then
    printf 'missing paths in %s:\n' "$DEB" >&2
    for path in "${missing[@]}"; do
        printf '  /%s\n' "$path" >&2
    done
    die "${#missing[@]} required path(s) missing from the package"
fi
printf '  paths: %d/%d required paths present\n' "$used_paths" "${#REQUIRED_PATHS[@]}"

# --- 4. validators over the installed-tree copies ---------------------------
SCRATCH="$(mktemp -d)" || die "mktemp -d failed"
dpkg-deb -x "$DEB" "$SCRATCH" || die "dpkg-deb -x failed"

desktop_file="$SCRATCH/usr/share/applications/io.github.wilkin.Segmenta.desktop"
metainfo_file="$SCRATCH/usr/share/metainfo/io.github.wilkin.Segmenta.metainfo.xml"
[ -f "$desktop_file" ] || die "extracted desktop file not found at $desktop_file"
[ -f "$metainfo_file" ] || die "extracted metainfo file not found at $metainfo_file"

command -v desktop-file-validate >/dev/null 2>&1 ||
    die "desktop-file-validate is not installed (install desktop-file-utils)"
desktop-file-validate "$desktop_file" ||
    die "desktop-file-validate rejected $(basename -- "$desktop_file")"
printf '  desktop-file-validate: ok\n'

command -v appstreamcli >/dev/null 2>&1 ||
    die "appstreamcli is not installed (install appstream)"
appstreamcli validate --no-net "$metainfo_file" ||
    die "appstreamcli validate rejected $(basename -- "$metainfo_file")"
printf '  appstreamcli validate: ok\n'

# --- 5. lintian (optional, informational) -----------------------------------
if command -v lintian >/dev/null 2>&1; then
    printf '  lintian: running (informational)\n'
    if lintian "$DEB"; then
        LINTIAN_STATUS="ok"
    else
        LINTIAN_STATUS="reported issues (non-blocking)"
        printf 'WARN: lintian reported issues; review them before publishing\n' >&2
    fi
else
    LINTIAN_STATUS="not installed, skipped"
    printf 'lintian: not installed, skipped\n'
fi

summary_ok
