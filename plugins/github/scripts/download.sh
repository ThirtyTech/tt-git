#!/bin/sh
# The plugin's build step: downloads the prebuilt tt-git-github for this
# plugin version and platform into bin/. Without a release for this
# version (a development checkout), it builds it with Go instead.
set -eu

version=$(sed -n 's/^version *= *"\(.*\)"/\1/p' tt-git-plugin.toml | head -n 1)
case "$(uname -s)" in
  Linux) os=linux ;;
  Darwin) os=darwin ;;
  *) echo "github: unsupported OS $(uname -s)" >&2; exit 1 ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) arch=amd64 ;;
  arm64 | aarch64) arch=arm64 ;;
  *) echo "github: unsupported CPU $(uname -m)" >&2; exit 1 ;;
esac
url="https://github.com/ThirtyTech/tt-git/releases/download/github-v$version/tt-git-github-$os-$arch"

mkdir -p bin
echo "Downloading $url"
if curl -fsSL -o bin/tt-git-github.part "$url"; then
  mv bin/tt-git-github.part bin/tt-git-github
  chmod +x bin/tt-git-github
elif command -v go >/dev/null 2>&1; then
  rm -f bin/tt-git-github.part
  echo "No release for github-v$version; building with Go"
  go build -o bin/tt-git-github .
else
  rm -f bin/tt-git-github.part
  echo "github: couldn't download $url, and Go isn't installed to build it" >&2
  exit 1
fi
