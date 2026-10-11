#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"

echo "Building Ghost Coach for Linux..."
npm run desktop
mkdir -p bin
install -m 0755 src-tauri/target/release/sonic-sales bin/sonic-sales
app_dir="$(pwd)"
applications_dir="${XDG_DATA_HOME:-${HOME}/.local/share}/applications"
mkdir -p "$applications_dir"
cat > "$applications_dir/sonic-sales.desktop" <<EOF
[Desktop Entry]
Version=1.0
Name=Ghost Coach
Comment=Live sales overlay coach
Exec=${app_dir}/start.sh
TryExec=${app_dir}/start.sh
Icon=${app_dir}/app-icon-1024.png
Terminal=false
Type=Application
Categories=Utility;X-Sales;
EOF
chmod 0755 "$applications_dir/sonic-sales.desktop"
echo "Build complete. Launch with ./start.sh"
