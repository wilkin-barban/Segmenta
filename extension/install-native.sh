#!/usr/bin/env bash
# Segmenta: install the native messaging manifests for the browsers found on
# this machine.
#
#   ./install-native.sh                 # detect browsers and write their manifest
#   ./install-native.sh /path/to/host   # explicit segmenta-host binary
#   ./install-native.sh --print         # print the documented paths, write nothing
#
# The Chromium family uses allowed_origins with the extension id derived from the
# stable manifest key; Firefox uses allowed_extensions with the gecko id.
set -euo pipefail

HOST_NAME="io.github.wilkin.segmenta"
DESCRIPTION="Host de mensajeria nativa para Segmenta"
# Single source of truth for the extension origin; the id is derived from it.
CHROMIUM_ORIGIN="chrome-extension://kdeldnifdbpemnhfpmhonlnibbhnoaan/"
EXTENSION_ID="${CHROMIUM_ORIGIN#chrome-extension://}"
EXTENSION_ID="${EXTENSION_ID%/}"
GECKO_ID="segmenta@wilkin.github.io"
DEFAULT_HOST_BIN="/usr/bin/segmenta-host"

PRINT_ONLY=0
HOST_BIN=""

usage() {
  cat <<'USAGE'
usage: install-native.sh [--print] [path-to-segmenta-host]

  --print   print the native manifest paths for every supported browser and exit
  -h,--help show this help
USAGE
}

for arg in "$@"; do
  case "$arg" in
    --print) PRINT_ONLY=1 ;;
    -h | --help)
      usage
      exit 0
      ;;
    -*)
      echo "install-native.sh: unknown option: $arg" >&2
      usage >&2
      exit 2
      ;;
    *)
      if [ -n "$HOST_BIN" ]; then
        echo "install-native.sh: only one host path is accepted" >&2
        exit 2
      fi
      HOST_BIN="$arg"
      ;;
  esac
done

CONFIG_HOME="${XDG_CONFIG_HOME:-${HOME}/.config}"

# browser-command:relative-config-dir:family
BROWSERS=(
  "google-chrome|google-chrome/NativeMessagingHosts|chromium"
  "chromium|chromium/NativeMessagingHosts|chromium"
  "chromium-browser|chromium/NativeMessagingHosts|chromium"
  "brave-browser|BraveSoftware/Brave-Browser/NativeMessagingHosts|chromium"
  "microsoft-edge|microsoft-edge/NativeMessagingHosts|chromium"
  "vivaldi|vivaldi/NativeMessagingHosts|chromium"
  "opera|opera/NativeMessagingHosts|chromium"
  "firefox|mozilla/native-messaging-hosts|firefox"
)

manifest_path() {
  local relative_dir="$1"
  echo "${CONFIG_HOME}/${relative_dir}/${HOST_NAME}.json"
}

print_all() {
  local entry command relative_dir family
  for entry in "${BROWSERS[@]}"; do
    IFS='|' read -r command relative_dir family <<<"$entry"
    printf '%-16s %-8s %s\n' "$command" "$family" "$(manifest_path "$relative_dir")"
  done
}

if [ "$PRINT_ONLY" -eq 1 ]; then
  print_all
  exit 0
fi

if [ -z "$HOST_BIN" ]; then
  HOST_BIN="$(command -v segmenta-host || true)"
fi

if [ -z "$HOST_BIN" ]; then
  echo "install-native.sh: segmenta-host was not found in PATH." >&2
  echo "install-native.sh: pass the absolute path as the first argument." >&2
  exit 1
fi

case "$HOST_BIN" in
  /*) ;;
  *)
    echo "install-native.sh: the host path must be absolute: $HOST_BIN" >&2
    exit 1
    ;;
esac

if [ ! -x "$HOST_BIN" ]; then
  echo "install-native.sh: the host binary is not executable: $HOST_BIN" >&2
  exit 1
fi

json_escape() {
  printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'
}

write_manifest() {
  local target="$1"
  local family="$2"
  local scratch dir
  dir="$(dirname "$target")"
  mkdir -p "$dir"
  scratch="$(mktemp "${TMPDIR:-/tmp}/segmenta-native-XXXXXX.json")"
  if [ "$family" = "firefox" ]; then
    cat >"$scratch" <<JSON
{
  "name": "${HOST_NAME}",
  "description": "$(json_escape "$DESCRIPTION")",
  "path": "$(json_escape "$HOST_BIN")",
  "type": "stdio",
  "allowed_extensions": ["${GECKO_ID}"]
}
JSON
  else
    cat >"$scratch" <<JSON
{
  "name": "${HOST_NAME}",
  "description": "$(json_escape "$DESCRIPTION")",
  "path": "$(json_escape "$HOST_BIN")",
  "type": "stdio",
  "allowed_origins": ["${CHROMIUM_ORIGIN}"]
}
JSON
  fi
  chmod 0644 "$scratch"
  mv -f "$scratch" "$target"
  echo "wrote ${target}"
}

installed=0
for entry in "${BROWSERS[@]}"; do
  IFS='|' read -r command relative_dir family <<<"$entry"
  if command -v "$command" >/dev/null 2>&1; then
    write_manifest "$(manifest_path "$relative_dir")" "$family"
    installed=$((installed + 1))
  fi
done

if [ "$installed" -eq 0 ]; then
  echo "install-native.sh: no supported browser was found in PATH." >&2
  echo "install-native.sh: run with --print to see the expected manifest paths." >&2
  exit 0
fi

echo "install-native.sh: ${installed} native manifest(s) written for ${HOST_BIN}"
