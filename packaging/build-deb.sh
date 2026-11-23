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

# Builds a Debian binary package.
#
# The packaging lives in `debian/` at the top of the tree, which is where
# `dpkg-buildpackage` insists on finding it, so this script is a thin wrapper
# rather than the real thing: it puts the toolchain on PATH, runs the build, and
# moves the artefacts out of the parent directory into `dist/` where the
# AppImage also lands.
#
# `-d` skips the build-dependency check. `debian/control` names `cargo` and
# `rustc` because that is what the build needs, but a machine with a rustup
# toolchain has neither as a Debian package, and the check would refuse to run
# on a machine that can obviously build the thing.
#
# Usage: packaging/build-deb.sh [output-dir]

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="${1:-$root/dist}"

export PATH=/opt/DEV/rust/cargo/bin:$PATH

cd "$root"

echo "==> building the package"
dpkg-buildpackage -b -us -uc -d

echo "==> collecting artefacts"
mkdir -p "$out"
version="$(dpkg-parsechangelog -S Version)"
arch="$(dpkg-architecture -q DEB_HOST_ARCH)"
mv -f "$root/../npp_${version}_${arch}.deb" "$out/"
mv -f "$root/../npp_${version}_${arch}.buildinfo" "$out/" 2>/dev/null || true
mv -f "$root/../npp_${version}_${arch}.changes" "$out/" 2>/dev/null || true

echo "==> done"
ls -lh "$out/npp_${version}_${arch}.deb"
