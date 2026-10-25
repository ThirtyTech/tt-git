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
#   tt-git-gui-<tag>-<platform>.deb      when NFPM names nfpm: packages that
#   tt-git-gui-<tag>-<platform>.rpm      install the same tree under /usr
#   tt-git-gui-<tag>-<platform>.pkg.tar.zst  (nfpm.yaml)
#
# All of them carry tt-git next to tt-git-gui, where the GUI looks for the
# binary it gives plugins as TTGIT_BIN_PATH.
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

case "$(uname -m)" in
    x86_64 | amd64) arch=x86_64 goarch=amd64 ;;
    aarch64 | arm64) arch=aarch64 goarch=arm64 ;;
    *) arch="" goarch="" ;;
esac

if [ -n "${APPIMAGETOOL:-}" ]; then
    # The AppImage: the same tree under usr/, with the .desktop file and icon
    # at the top and AppRun starting tt-git-gui.
    [ -n "$arch" ] || { echo "package.sh: no AppImage for $(uname -m)" >&2; exit 1; }
    appdir="$stage/tt-git.AppDir"
    mkdir -p "$appdir/usr"
    cp -a "$tree/bin" "$tree/share" "$appdir/usr/"
    cp "$here/tt-git.desktop" "$appdir/tt-git.desktop"
    cp "$here/../icons/256.png" "$appdir/tt-git.png"
    ln -s tt-git.png "$appdir/.DirIcon"
    ln -s usr/bin/tt-git-gui "$appdir/AppRun"
    # APPIMAGE_EXTRACT_AND_RUN: appimagetool is itself an AppImage, and CI
    # runners have no FUSE to mount it with.
    ARCH=$arch VERSION="${tag#v}" APPIMAGE_EXTRACT_AND_RUN=1 \
        "$APPIMAGETOOL" --no-appstream "$appdir" "$name.AppImage"
    echo "Wrote $name.AppImage"
fi

if [ -n "${NFPM:-}" ]; then
    # The packages, from the same tree. nfpm reads the tree relative to the
    # directory it runs in.
    [ -n "$goarch" ] || { echo "package.sh: no packages for $(uname -m)" >&2; exit 1; }
    out=$(pwd)
    for format in deb rpm archlinux; do
        version="${tag#v}"
        case "$format" in
            # nfpm drops a pre-release from Arch's pkgver, which can't hold
            # a "-": 0.9.0-rc.1 goes in as 0.9.0rc.1, which pacman orders
            # before 0.9.0. deb and rpm get 0.9.0~rc.1 from nfpm.
            archlinux) file="$name.pkg.tar.zst" version="${version/-/}" ;;
            *) file="$name.$format" ;;
        esac
        (cd "$tree" && VERSION="$version" ARCH=$goarch \
            "$NFPM" package --config "$here/nfpm.yaml" --packager "$format" --target "$out/$file")
        echo "Wrote $file"
    done
fi
