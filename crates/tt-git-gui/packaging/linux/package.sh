#!/usr/bin/env bash
# Packages tt-git-gui for Linux:
#
#   package.sh <bin-dir> <tag> <platform>
#
# <bin-dir> holds release builds of tt-git-gui and tt-git, <tag> is the
# release tag (v0.9.0) and <platform> the release's name for the target
# (linux-x64). Writes, in the current directory:
#
#   tt-git-gui-<tag>-<platform>.tar.gz   a prefix tree to unpack into ~/.local
#                                        or /usr/local (bin/, share/)
#   tt-git-gui-<tag>-<platform>.AppImage when APPIMAGETOOL names appimagetool
#
# Both carry tt-git next to tt-git-gui, where the GUI looks for the binary
# it gives plugins as TTGIT_BIN_PATH.
set -euo pipefail

[ $# -eq 3 ] || { echo "usage: $0 <bin-dir> <tag> <platform>" >&2; exit 2; }
bin_dir=$1 tag=$2 platform=$3
here=$(cd "$(dirname "$0")" && pwd)
crate=$(cd "$here/../.." && pwd)
root=$(cd "$crate/../.." && pwd)
name="tt-git-gui-$tag-$platform"
sizes=(16 32 48 64 128 256 512)

for exe in tt-git-gui tt-git; do
    [ -x "$bin_dir/$exe" ] || { echo "package.sh: no $exe in $bin_dir" >&2; exit 1; }
done

# The prefix tree: bin/, share/applications, share/icons/hicolor, share/doc.
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
tree="$stage/$name"
install -Dm755 "$bin_dir/tt-git-gui" "$tree/bin/tt-git-gui"
install -Dm755 "$bin_dir/tt-git" "$tree/bin/tt-git"
install -Dm644 "$here/tt-git.desktop" "$tree/share/applications/tt-git.desktop"
for size in "${sizes[@]}"; do
    install -Dm644 "$here/../icons/$size.png" "$tree/share/icons/hicolor/${size}x${size}/apps/tt-git.png"
done
install -Dm644 "$crate/assets/icon.svg" "$tree/share/icons/hicolor/scalable/apps/tt-git.svg"
install -Dm644 "$root/README.md" "$tree/share/doc/tt-git/README.md"
for license in "$root"/LICENSE*; do
    [ -f "$license" ] && install -Dm644 "$license" "$tree/share/doc/tt-git/$(basename "$license")"
done
tar -czf "$name.tar.gz" -C "$stage" "$name"
echo "Wrote $name.tar.gz"

[ -n "${APPIMAGETOOL:-}" ] || exit 0

# The AppImage: the same tree under usr/, with the .desktop file and icon at
# the top and AppRun starting tt-git-gui.
appdir="$stage/tt-git.AppDir"
mkdir -p "$appdir/usr"
cp -a "$tree/bin" "$tree/share" "$appdir/usr/"
cp "$here/tt-git.desktop" "$appdir/tt-git.desktop"
cp "$here/../icons/256.png" "$appdir/tt-git.png"
ln -s tt-git.png "$appdir/.DirIcon"
ln -s usr/bin/tt-git-gui "$appdir/AppRun"
case "$(uname -m)" in
    x86_64 | amd64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *) echo "package.sh: no AppImage for $(uname -m)" >&2; exit 1 ;;
esac
# APPIMAGE_EXTRACT_AND_RUN: appimagetool is itself an AppImage, and CI
# runners have no FUSE to mount it with.
ARCH=$arch VERSION="${tag#v}" APPIMAGE_EXTRACT_AND_RUN=1 \
    "$APPIMAGETOOL" --no-appstream "$appdir" "$name.AppImage"
echo "Wrote $name.AppImage"
