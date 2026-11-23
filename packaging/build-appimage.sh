#!/usr/bin/env bash
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

# Builds a self-contained AppImage.
#
# An AppImage is a squashfs image with a fixed layout: `AppRun` at the root, the
# program under `usr/bin`, a `.desktop` file and an icon beside it. The runtime
# mounts the image and runs `AppRun`, so nothing is installed and nothing is
# unpacked — the file is the program.
#
# The binary is statically linked against everything it can be. What it cannot
# avoid is the graphics stack: Vulkan, X11, and the C library are the host's
# business, and every desktop that can run a game can run this.
#
# Usage: packaging/build-appimage.sh [output-dir]

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="${1:-$root/dist}"
here="$root/packaging"
appdir="$root/target/appimage/npp.AppDir"

export PATH=/opt/DEV/rust/cargo/bin:$PATH

echo "==> building release binary"
cargo build --release --manifest-path "$root/Cargo.toml" --bin npp

echo "==> assembling AppDir"
rm -rf "$appdir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/applications" \
    "$appdir/usr/share/icons/hicolor/256x256/apps" \
    "$appdir/usr/share/licenses/npp"
install -m 755 "$root/target/release/npp" "$appdir/usr/bin/npp"
install -m 755 "$here/AppRun" "$appdir/AppRun"
install -m 644 "$here/npp.desktop" "$appdir/npp.desktop"
install -m 644 "$here/npp.desktop" "$appdir/usr/share/applications/npp.desktop"
install -m 644 "$here/npp-256.png" "$appdir/npp.png"
install -m 644 "$here/npp-256.png" \
    "$appdir/usr/share/icons/hicolor/256x256/apps/npp.png"
install -m 644 "$root/LICENSE" "$appdir/usr/share/licenses/npp/LICENSE"
ln -sf npp.png "$appdir/.DirIcon"

echo "==> fetching appimagetool"
tool="$root/target/appimagetool-x86_64.AppImage"
if [ ! -x "$tool" ]; then
    curl -sSL -o "$tool" \
        https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
    chmod +x "$tool"
fi

echo "==> packing"
mkdir -p "$out"
# --appimage-extract-and-run avoids needing FUSE, which is often missing in
# containers and on minimal desktops.
ARCH=x86_64 "$tool" --appimage-extract-and-run \
    --no-appstream \
    "$appdir" "$out/npp-x86_64.AppImage"

echo "==> done"
ls -lh "$out/npp-x86_64.AppImage"
