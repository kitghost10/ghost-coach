#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"

if [ ! -f "bin/sonic-sales" ]; then
  echo "Building Ghost Coach so it can open without a local server..."
  npm run desktop
  mkdir -p bin
  install -m 0755 "src-tauri/target/release/sonic-sales" "bin/sonic-sales"
fi

# Wayland has no global window coordinates. Prefer XWayland when available so
# the coach can honor its top/bottom placement; an explicit user choice wins.
if [[ -n "${WAYLAND_DISPLAY:-}" && -n "${DISPLAY:-}" && -z "${GDK_BACKEND:-}" ]]; then
  export GDK_BACKEND=x11
fi

exec ./bin/sonic-sales "$@"
