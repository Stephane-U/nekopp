#!/bin/sh
# Builds every plugin and installs it where the editor reads plugins from.
#
# The editor looks in `<config>/npp/plugins`, one `.wasm` per plugin, and reloads
# that directory when a plugin is imported -- but not when one changes on disk,
# so an editor that is already running has to be restarted to see a rebuild.
set -eu
cd "$(dirname "$0")"

target=wasm32-unknown-unknown
cargo build --release --target "$target"

# XDG_CONFIG_HOME first, because that is what the editor itself honours.
dest="${XDG_CONFIG_HOME:-$HOME/.config}/npp/plugins"
mkdir -p "$dest"
for wasm in target/$target/release/npp_*.wasm; do
    install -m 644 "$wasm" "$dest/"
done
printf 'installed into %s:\n' "$dest"
ls -1 "$dest"
