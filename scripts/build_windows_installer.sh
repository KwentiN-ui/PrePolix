#!/usr/bin/env bash
# Builds the Windows installer for prepolix on Linux: cross-compiles prepolix.exe for
# x86_64-pc-windows-gnu with MinGW, fetches the Gmsh DLL and packs everything with NSIS.
#
#     scripts/build_windows_installer.sh
#
# Needs (Debian/Ubuntu): sudo apt install gcc-mingw-w64-x86-64 nsis python3
# and the Rust target:   rustup target add x86_64-pc-windows-gnu
#
# The installer ends up in target/windows-installer/prepolix-<version>-setup.exe.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
target=x86_64-pc-windows-gnu
out=target/windows-installer
stage="$out/stage"
gmsh=target/gmsh-windows

for tool in makensis x86_64-w64-mingw32-gcc python3; do
    command -v "$tool" >/dev/null || {
        echo "$tool fehlt, siehe Kopf von $0" >&2
        exit 1
    }
done
rustup target list --installed 2>/dev/null | grep -qx "$target" || rustup target add "$target"

version="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)"
[ -n "$version" ] || { echo "Version in Cargo.toml nicht gefunden" >&2; exit 1; }

cargo build --release --locked -p plx-app --target "$target"
[ -f "$gmsh/gmsh-4.15.dll" ] || python3 scripts/fetch_gmsh.py --platform windows --dest "$gmsh"

rm -rf "$stage"
mkdir -p "$stage/licenses"
cp "target/$target/release/prepolix.exe" "$stage/"
cp "$gmsh/gmsh-4.15.dll" crates/plx-app/assets/icon/prepolix.ico installer/windows/THIRD-PARTY.txt "$stage/"
cp LICENSE "$stage/LICENSE.txt"
cp "$gmsh/GMSH-LICENSE.txt" "$gmsh/GMSH-CREDITS.txt" installer/windows/licenses/*.txt "$stage/licenses/"
cp crates/plx-app/assets/fonts/OFL.txt "$stage/licenses/NotoSans-OFL.txt"
commit="$(git rev-parse HEAD 2>/dev/null || echo unbekannt)"
dirty="$(git status --porcelain --untracked-files=no 2>/dev/null | head -1)"
{
    echo "prepolix $version"
    echo "Quelltext: https://github.com/KwentiN-ui/prepolix"
    echo "Commit:    $commit${dirty:+ (mit lokalen Änderungen)}"
} >"$stage/SOURCE.txt"
# Windows editors expect CRLF line ends in the text files.
for file in "$stage"/*.txt "$stage"/licenses/*.txt; do
    sed -i 's/\r$//; s/$/\r/' "$file"
done

installer="$out/prepolix-$version-setup.exe"
makensis -V2 -DVERSION="$version" -DSTAGE="$root/$stage" -DOUTFILE="$root/$installer" \
    installer/windows/prepolix.nsi
echo "Installer: $installer ($(du -h "$installer" | cut -f1))"
