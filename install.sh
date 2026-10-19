#!/bin/sh
# Installs tt-git from its GitHub releases on Linux (x64, arm64) and macOS
# (Apple Silicon):
#
#   curl -fsSL https://github.com/ThirtyTech/tt-git/releases/latest/download/install.sh | sh
#
# Environment:
#   TTGIT_VERSION      tag to install, such as v0.2.0 (default: latest release)
#   TTGIT_INSTALL_DIR  where to put the binary (default: ~/.local/bin)

set -eu

repo="ThirtyTech/tt-git"
install_dir="${TTGIT_INSTALL_DIR:-$HOME/.local/bin}"

err() {
    echo "tt-git install: $*" >&2
    exit 1
}

command -v curl >/dev/null 2>&1 || err "curl is required"
command -v tar >/dev/null 2>&1 || err "tar is required"

case "$(uname -s)" in
    Linux) os=linux ;;
    Darwin) os=macos ;;
    *) err "unsupported OS: $(uname -s). On Windows, download the zip from https://github.com/$repo/releases" ;;
esac

case "$(uname -m)" in
    x86_64 | amd64) arch=x64 ;;
    aarch64 | arm64) arch=arm64 ;;
    *) err "unsupported architecture: $(uname -m)" ;;
esac

if [ "$os" = macos ] && [ "$arch" = x64 ]; then
    err "Intel Macs aren't supported; only Apple Silicon builds are published"
fi

if [ -n "${TTGIT_VERSION:-}" ]; then
    version="$TTGIT_VERSION"
else
    # /releases/latest redirects to /releases/tag/<version>.
    version=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$repo/releases/latest")
    version="${version##*/}"
    [ -n "$version" ] && [ "$version" != latest ] || err "couldn't find the latest release"
fi

name="tt-git-$version-$os-$arch"
base="https://github.com/$repo/releases/download/$version"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "Downloading tt-git $version ($os-$arch)"
curl -fsSL -o "$tmp/$name.tar.gz" "$base/$name.tar.gz" || err "download failed: $base/$name.tar.gz"
curl -fsSL -o "$tmp/SHA256SUMS.txt" "$base/SHA256SUMS.txt" || err "couldn't download SHA256SUMS.txt"

expected=$(grep " $name.tar.gz\$" "$tmp/SHA256SUMS.txt" | cut -d' ' -f1)
[ -n "$expected" ] || err "no checksum for $name.tar.gz"
if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$tmp/$name.tar.gz" | cut -d' ' -f1)
else
    actual=$(shasum -a 256 "$tmp/$name.tar.gz" | cut -d' ' -f1)
fi
[ "$expected" = "$actual" ] || err "checksum mismatch for $name.tar.gz"

tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
mkdir -p "$install_dir"
# Install through a temp name and rename, so a running tt-git isn't overwritten in place.
cp "$tmp/$name/tt-git" "$install_dir/.tt-git.new"
chmod 755 "$install_dir/.tt-git.new"
mv -f "$install_dir/.tt-git.new" "$install_dir/tt-git"

echo "Installed tt-git $version to $install_dir/tt-git"

case ":$PATH:" in
    *":$install_dir:"*) ;;
    *) echo "Note: $install_dir is not on your PATH. Add it with:
  export PATH=\"$install_dir:\$PATH\"" ;;
esac
