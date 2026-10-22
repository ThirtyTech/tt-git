#!/bin/sh
# The plugin's build step: downloads the prebuilt tt-git-ado for this
# plugin version and platform into bin/. Without a release for this
# version (a development checkout), it builds it with Go instead.
set -eu

version=$(sed -n 's/^version *= *"\(.*\)"/\1/p' tt-git-plugin.toml | head -n 1)
case "$(uname -s)" in
  Linux) os=linux ;;
  Darwin) os=darwin ;;
  *) echo "azure-devops: unsupported OS $(uname -s)" >&2; exit 1 ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) arch=amd64 ;;
  arm64 | aarch64) arch=arm64 ;;
  *) echo "azure-devops: unsupported CPU $(uname -m)" >&2; exit 1 ;;
esac
url="https://github.com/ThirtyTech/tt-git/releases/download/azure-devops-v$version/tt-git-ado-$os-$arch"

mkdir -p bin
echo "Downloading $url"
if curl -fsSL -o bin/tt-git-ado.part "$url"; then
  mv bin/tt-git-ado.part bin/tt-git-ado
  chmod +x bin/tt-git-ado
elif command -v go >/dev/null 2>&1; then
  rm -f bin/tt-git-ado.part
  echo "No release for azure-devops-v$version; building with Go"
  go build -o bin/tt-git-ado .
else
  rm -f bin/tt-git-ado.part
  echo "azure-devops: couldn't download $url, and Go isn't installed to build it" >&2
  exit 1
fi
