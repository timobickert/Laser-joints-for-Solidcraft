#!/usr/bin/env bash
# Build the plugin into dist/solidcraft-laserjoints/ and dist/solidcraft-laserjoints.zip.
# Needs: Rust with the wasm32-unknown-unknown target, and wasm-tools
# (https://github.com/bytecodealliance/wasm-tools).
set -euo pipefail
cd "$(dirname "$0")"
rustup target add wasm32-unknown-unknown
cargo build --release --lib --target wasm32-unknown-unknown
core="${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/release/solidcraft_laserjoints.wasm"
out=dist/solidcraft-laserjoints
rm -rf "$out"
mkdir -p "$out"
if [[ -n "${WASM_TOOLS:-}" ]]; then
  wasm_tools="$WASM_TOOLS"
elif command -v wasm-tools >/dev/null 2>&1; then
  wasm_tools=$(command -v wasm-tools)
elif [[ -x tools/wasm-tools ]]; then
  wasm_tools=tools/wasm-tools
elif [[ -x tools/wasm-tools.exe ]]; then
  wasm_tools=tools/wasm-tools.exe
else
  echo "wasm-tools nicht gefunden. Lege es auf PATH, setze WASM_TOOLS oder kopiere es nach tools/." >&2
  exit 1
fi
"$wasm_tools" component new "$core" -o "$out/plugin.wasm"
cp plugin.json README.md LICENSE "$out/"
rm -f dist/solidcraft-laserjoints.zip
(
  cd "$out"
  zip -q -r ../solidcraft-laserjoints.zip .
)
echo "built $out"
echo "built dist/solidcraft-laserjoints.zip"
