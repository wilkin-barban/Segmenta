//! Helpers shared by the daemon integration tests.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Write a `secret-tool` stand-in that keeps its data inside `dir`.
///
/// Pointing `SEGMENTA_SECRET_TOOL` at a missing binary is not enough: the daemon then has no
/// secret backend at all and every token write fails. This fake behaves like the real CLI but
/// never touches the user's keyring, so the tests exercise the same code path in isolation.
pub fn fake_secret_tool(dir: &Path) -> PathBuf {
    let script = dir.join("secret-tool");
    let state = dir.join("secret-state");
    let body = format!(
        r#"#!/bin/sh
state="{state}"
cmd="$1"; shift || true
case "$cmd" in
  --version) exit 2 ;;
  lookup)
    key=""
    while [ $# -gt 0 ]; do
      case "$1" in key) key=$(printf %s "$2" | tr "/" "_"); shift 2 ;; *) shift ;; esac
    done
    if [ -f "$state/$key" ]; then cat "$state/$key"; exit 0; fi
    exit 1 ;;
  store)
    key=""
    while [ $# -gt 0 ]; do
      case "$1" in key) key=$(printf %s "$2" | tr "/" "_"); shift 2 ;; *) shift ;; esac
    done
    mkdir -p "$state"
    cat > "$state/$key"
    exit 0 ;;
  clear)
    key=""
    while [ $# -gt 0 ]; do
      case "$1" in key) key=$(printf %s "$2" | tr "/" "_"); shift 2 ;; *) shift ;; esac
    done
    rm -f "$state/$key"
    exit 0 ;;
esac
exit 1
"#,
        state = state.display()
    );
    std::fs::write(&script, body).unwrap();
    let mut permissions = std::fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script, permissions).unwrap();
    script
}
