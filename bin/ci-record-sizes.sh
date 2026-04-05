#!/usr/bin/env sh
#-*-mode: Shell-script; coding: utf-8;-*-
# Simulates the CI workflow locally without committing.
# Downloads the latest pre-release binaries from GitHub, builds the dep graph,
# and runs ci-record-sizes — all outside the nix sandbox so network access works.
#
# Usage: ./bin/test-ci
# Description: Unified versions script to handle listing latest versions,
# firmware, and updating of versions.
_base=$(basename "$0")
_dir=$(cd -P -- "$(dirname -- "$(command -v -- "$0")")" && pwd -P || exit 126)
export _base _dir

set "${SETOPTS:--eu}"

PREFIX=$(mktemp -d)
trap 'rm -rf "$PREFIX"' EXIT INT TERM QUIT

# Build the dep graph first — fail fast before wasting time on downloads.
echo "Building dependency graph..."
svg=$(nix build --no-link --print-out-paths .#ci-pugio-graph)/deps.svg
deps="${PREFIX}/deps.svg"
install -m400 "${svg}" "${deps}"

# Mirror the artifact dirs the gh workflow produces.
mkdir -p "${PREFIX}/artifacts/mitchty-wasm"
mkdir -p "${PREFIX}/artifacts/mitchty-windows-x86_64"
mkdir -p "${PREFIX}/artifacts/mitchty-darwin-aarch64"

wasm="${PREFIX}/artifacts/mitchty-wasm"
win="${PREFIX}/artifacts/mitchty-windows-x86_64/mitchty-windows-x86_64.exe"
mac="${PREFIX}/artifacts/mitchty-darwin-aarch64/mitchty-darwin-aarch64"

echo "Downloading latest pre-release binaries..."
ASSETS=$(curl -s https://api.github.com/repos/mitchty/mitchty.github.io/releases | jq -r '.[0].assets[].browser_download_url')

curl -fL "$(echo "$ASSETS" | awk '/mitchty-wasm\.tar\.gz/')" | tar -xzf - -C "${wasm}"
curl -fL "$(echo "$ASSETS" | awk '/mitchty-windows-x86_64\.exe/')" -o "${win}"
curl -fL "$(echo "$ASSETS" | awk '/mitchty-darwin-aarch64/')" -o "${mac}"

"${_dir}/record-sizes.sh" "${wasm}/mitchty_bg.wasm" "${win}" "${mac}" "${deps}"
