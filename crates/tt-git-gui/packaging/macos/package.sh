#!/usr/bin/env bash
# Packages tt-git-gui for macOS:
#
#   package.sh <bin-dir> <tag> <platform>
#
# <bin-dir> holds release builds of tt-git-gui and tt-git, <tag> is the
# release tag (v0.9.0) and <platform> the release's name for the target
# (macos-arm64). Writes tt-git-gui-<tag>-<platform>.dmg in the current
# directory, holding tt-git.app and a link to /Applications over a
# background (dmg.py, which needs python3 and network access for pip to
# fetch dmg-requirements.txt). tt-git sits in
# tt-git.app/Contents/MacOS next to tt-git-gui, where the GUI looks for the
# binary it gives plugins as TTGIT_BIN_PATH.
#
# Signing, from the environment (the release workflow passes its secrets):
#
#   MACOS_CERT_P12          base64 of a "Developer ID Application"
#                           certificate and key, exported as .p12
#   MACOS_CERT_PASSWORD     the .p12's password
#   MACOS_SIGNING_IDENTITY  optional; the identity to sign with. Default:
#                           the first Developer ID Application identity in
#                           the certificate
#   MACOS_NOTARY_APPLE_ID   the Apple ID to notarize with
#   MACOS_NOTARY_TEAM_ID    its team ID
#   MACOS_NOTARY_PASSWORD   an app-specific password for that Apple ID
#
# Without MACOS_CERT_P12 the app is signed ad hoc (codesign -s -), which
# Apple Silicon needs to run it at all; Gatekeeper then blocks the first
# launch until the user allows it. Without the three MACOS_NOTARY_*
# values a Developer ID build is signed but not notarized.
set -euo pipefail

[ $# -eq 3 ] || { echo "usage: $0 <bin-dir> <tag> <platform>" >&2; exit 2; }
bin_dir=$1 tag=$2 platform=$3
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../.." && pwd)
name="tt-git-gui-$tag-$platform"
version="${tag#v}"

for exe in tt-git-gui tt-git; do
    [ -x "$bin_dir/$exe" ] || { echo "package.sh: no $exe in $bin_dir" >&2; exit 1; }
done

stage=$(mktemp -d)
keychain=""
cleanup() {
    [ -n "$keychain" ] && security delete-keychain "$keychain" 2>/dev/null
    rm -rf "$stage"
}
trap cleanup EXIT

# The bundle.
app="$stage/dmg/tt-git.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
sed "s/@VERSION@/$version/g" "$here/Info.plist" > "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"
printf 'APPL????' > "$app/Contents/PkgInfo"
cp "$bin_dir/tt-git-gui" "$bin_dir/tt-git" "$app/Contents/MacOS/"
cp "$root/README.md" "$app/Contents/Resources/"

# The icon, from the committed PNGs (render-icons.sh).
iconset="$stage/tt-git.iconset"
mkdir "$iconset"
for size in 16 32 128 256 512; do
    cp "$here/../icons/$size.png" "$iconset/icon_${size}x${size}.png"
    cp "$here/../icons/$((size * 2)).png" "$iconset/icon_${size}x${size}@2x.png"
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/tt-git.icns"

# Signing: the inner tt-git first, then the bundle (which seals tt-git-gui
# and the resources). The hardened runtime and a secure timestamp are what
# notarization asks for.
identity="-"
if [ -n "${MACOS_CERT_P12:-}" ]; then
    keychain="$stage/signing.keychain-db"
    keychain_password=$(uuidgen)
    echo "$MACOS_CERT_P12" | base64 --decode > "$stage/cert.p12"
    security create-keychain -p "$keychain_password" "$keychain"
    security set-keychain-settings -lut 21600 "$keychain"
    security unlock-keychain -p "$keychain_password" "$keychain"
    security import "$stage/cert.p12" -k "$keychain" -P "${MACOS_CERT_PASSWORD:-}" \
        -T /usr/bin/codesign -T /usr/bin/security
    rm "$stage/cert.p12"
    security set-key-partition-list -S apple-tool:,apple: -s -k "$keychain_password" "$keychain" >/dev/null
    # Search the new keychain as well as the usual ones.
    security list-keychains -d user -s "$keychain" $(security list-keychains -d user | tr -d '"')
    identity="${MACOS_SIGNING_IDENTITY:-}"
    if [ -z "$identity" ]; then
        identity=$(security find-identity -v -p codesigning "$keychain" |
            sed -n 's/.*"\(Developer ID Application:[^"]*\)".*/\1/p' | head -1)
    fi
    [ -n "$identity" ] || { echo "package.sh: no Developer ID Application identity in MACOS_CERT_P12" >&2; exit 1; }
    echo "Signing with $identity"
    sign=(codesign --force --options runtime --timestamp --sign "$identity")
else
    echo "No MACOS_CERT_P12: signing ad hoc"
    sign=(codesign --force --sign -)
fi
"${sign[@]}" "$app/Contents/MacOS/tt-git"
"${sign[@]}" "$app"
codesign --verify --strict --verbose=2 "$app"

# The disk image: the app and a link to drag it onto, laid out over
# dmg-background.png by dmg.py with the pinned dmgbuild.
python3 -m venv "$stage/venv"
"$stage/venv/bin/pip" install --quiet --disable-pip-version-check --require-hashes \
    -r "$here/dmg-requirements.txt"
rm -f "$name.dmg"
"$stage/venv/bin/python" "$here/dmg.py" "$app" "$name.dmg"

if [ "$identity" != "-" ]; then
    codesign --force --timestamp --sign "$identity" "$name.dmg"
    if [ -n "${MACOS_NOTARY_APPLE_ID:-}" ] && [ -n "${MACOS_NOTARY_TEAM_ID:-}" ] && [ -n "${MACOS_NOTARY_PASSWORD:-}" ]; then
        echo "Notarizing $name.dmg"
        notary=(--apple-id "$MACOS_NOTARY_APPLE_ID" --team-id "$MACOS_NOTARY_TEAM_ID"
            --password "$MACOS_NOTARY_PASSWORD")
        xcrun notarytool submit "$name.dmg" "${notary[@]}" --wait --timeout 30m \
            --output-format json > "$stage/notary.json" || true
        cat "$stage/notary.json"
        status=$(plutil -extract status raw "$stage/notary.json" 2>/dev/null || echo unknown)
        if [ "$status" != Accepted ]; then
            # Apple's log says which file failed and why.
            id=$(plutil -extract id raw "$stage/notary.json" 2>/dev/null || true)
            [ -n "$id" ] && xcrun notarytool log "$id" "${notary[@]}" || true
            echo "package.sh: notarization ended with status $status" >&2
            exit 1
        fi
        xcrun stapler staple "$name.dmg"
        xcrun stapler validate "$name.dmg"
    else
        echo "No MACOS_NOTARY_* credentials: signed but not notarized"
    fi
fi
echo "Wrote $name.dmg"
