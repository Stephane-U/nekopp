#!/bin/sh
# Neko++ -- a GPU-rendered text editor.
# Copyright (C) 2026 Stéphane Urdy
#
# This program is free software: you can redistribute it and/or modify it under
# the terms of the GNU General Public License as published by the Free Software
# Foundation, version 3 of the License.
#
# This program is distributed in the hope that it will be useful, but WITHOUT
# ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
# FOR A PARTICULAR PURPOSE. See the GNU General Public License for more
# details.
#
# You should have received a copy of the GNU General Public License along with
# this program. If not, see <https://www.gnu.org/licenses/>.

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
