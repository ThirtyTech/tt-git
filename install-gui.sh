#!/bin/sh
# Installs the tt-git desktop app (tt-git-gui) from its GitHub releases on
# Linux (x64, arm64):
#
#   curl -fsSL https://github.com/ThirtyTech/tt-git/releases/latest/download/install-gui.sh | sh
#
# It unpacks the release's tt-git-gui-<version>-linux-<arch>.tar.gz into a
# prefix: tt-git-gui and tt-git in bin/, tt-git.desktop in
# share/applications/, the icons in share/icons/hicolor/ and the README in
# share/doc/tt-git/. To remove them again:
#
#   curl -fsSL https://github.com/ThirtyTech/tt-git/releases/latest/download/install-gui.sh | sh -s -- --uninstall
#
# Environment:
#   TTGIT_VERSION  tag to install, such as v0.10.0 (default: latest release)
#   TTGIT_PREFIX   where to install (default: ~/.local). For every user, use
#                  /usr/local and run the script with sudo.

set -eu

repo="ThirtyTech/tt-git"
prefix="${TTGIT_PREFIX:-$HOME/.local}"
uninstall=0

err() {
    echo "tt-git install: $*" >&2
    exit 1
}

for arg in "$@"; do
    case "$arg" in
        --uninstall) uninstall=1 ;;
        *) err "unknown option: $arg (the only one is --uninstall)" ;;
    esac
done

# The files a release's tarball installs, relative to the prefix.
sizes="16 32 48 64 128 256 512"
files() {
    echo bin/tt-git-gui
    echo bin/tt-git
    echo share/applications/tt-git.desktop
    for size in $sizes; do
        echo "share/icons/hicolor/${size}x${size}/apps/tt-git.png"
    done
    echo share/icons/hicolor/scalable/apps/tt-git.svg
}

# Tells the desktop about new or removed launchers and icons. Desktops that
# watch these folders don't need it; the tools may not be installed.
refresh() {
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database -q "$prefix/share/applications" 2>/dev/null || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1 && [ -f "$prefix/share/icons/hicolor/index.theme" ]; then
        gtk-update-icon-cache -q -t "$prefix/share/icons/hicolor" 2>/dev/null || true
    fi
}

[ "$(uname -s)" = Linux ] || err "this script is for Linux. On macOS, open the .dmg; on Windows, run the -setup.exe; both are at https://github.com/$repo/releases"

if [ "$uninstall" = 1 ]; then
    removed=0
    for f in $(files); do
        if [ -e "$prefix/$f" ]; then
            rm -f "$prefix/$f"
            removed=1
        fi
    done
    rm -rf "$prefix/share/doc/tt-git"
    [ "$removed" = 1 ] || err "tt-git-gui isn't installed in $prefix"
    refresh
    echo "Removed tt-git-gui and tt-git from $prefix. Settings in ~/.config/tt-git are kept."
    exit 0
fi

command -v curl >/dev/null 2>&1 || err "curl is required"
command -v tar >/dev/null 2>&1 || err "tar is required"
command -v sha256sum >/dev/null 2>&1 || err "sha256sum is required"

case "$(uname -m)" in
    x86_64 | amd64) arch=x64 ;;
    aarch64 | arm64) arch=arm64 ;;
    *) err "unsupported architecture: $(uname -m)" ;;
esac

if [ -n "${TTGIT_VERSION:-}" ]; then
    version="$TTGIT_VERSION"
else
    # /releases/latest redirects to /releases/tag/<version>.
    version=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$repo/releases/latest")
    version="${version##*/}"
    [ -n "$version" ] && [ "$version" != latest ] || err "couldn't find the latest release"
fi

name="tt-git-gui-$version-linux-$arch"
base="https://github.com/$repo/releases/download/$version"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "Downloading tt-git-gui $version (linux-$arch)"
curl -fsSL -o "$tmp/$name.tar.gz" "$base/$name.tar.gz" || err "download failed: $base/$name.tar.gz"
curl -fsSL -o "$tmp/SHA256SUMS.txt" "$base/SHA256SUMS.txt" || err "couldn't download SHA256SUMS.txt"

expected=$(grep " $name.tar.gz\$" "$tmp/SHA256SUMS.txt" | cut -d' ' -f1)
[ -n "$expected" ] || err "no checksum for $name.tar.gz"
actual=$(sha256sum "$tmp/$name.tar.gz" | cut -d' ' -f1)
[ "$expected" = "$actual" ] || err "checksum mismatch for $name.tar.gz"

tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
src="$tmp/$name"
[ -x "$src/bin/tt-git-gui" ] || err "$name.tar.gz has no bin/tt-git-gui"

mkdir -p "$prefix" || err "can't create $prefix"
[ -w "$prefix" ] || err "can't write to $prefix; run the script with sudo, or set TTGIT_PREFIX"
for f in $(files); do
    mkdir -p "$(dirname "$prefix/$f")"
    # Through a temp name and rename, so a running tt-git-gui or tt-git
    # isn't overwritten in place.
    cp "$src/$f" "$prefix/$f.new"
    mv -f "$prefix/$f.new" "$prefix/$f"
done
# The launcher starts tt-git-gui by its full path, so it works whether or
# not <prefix>/bin is on the session's PATH.
desktop="$prefix/share/applications/tt-git.desktop"
sed "s|^Exec=tt-git-gui |Exec=\"$prefix/bin/tt-git-gui\" |" "$desktop" > "$desktop.new"
mv -f "$desktop.new" "$desktop"
mkdir -p "$prefix/share/doc/tt-git"
cp "$src"/share/doc/tt-git/* "$prefix/share/doc/tt-git/"
refresh

echo "Installed tt-git-gui $version to $prefix/bin/tt-git-gui, with tt-git next to it"
echo "Start it from your app launcher (tt-git), or run tt-git-gui in a repository."

# The libraries tt-git-gui links. The ones it loads when it starts (Vulkan,
# Wayland) don't show here.
if command -v ldd >/dev/null 2>&1; then
    missing=$(ldd "$prefix/bin/tt-git-gui" 2>/dev/null | awk '/not found/ { printf "%s%s", sep, $1; sep = " " }')
    if [ -n "$missing" ]; then
        echo "Warning: tt-git-gui needs libraries that aren't installed: $missing"
        echo "  See https://thirtytech.github.io/tt-git/desktop/install/#linux for the packages."
    fi
fi

case ":$PATH:" in
    *":$prefix/bin:"*) ;;
    *) echo "Note: $prefix/bin is not on your PATH. To run tt-git-gui and tt-git from a terminal, add it:
  export PATH=\"$prefix/bin:\$PATH\"" ;;
esac
