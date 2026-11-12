#!/usr/bin/env bash
# Makes Adelite.app for Apple silicon from a release build and TDLib, and packs
# it twice: a .dmg for people (the app beside a link to Applications) and a
# .tar.gz for the updater (Adelite.app at its top).
#
# Runs on macOS (sips, iconutil, codesign, hdiutil):
#
#   bash packaging/macos/bundle.sh <version> <adelite binary> <libtdjson.dylib> <out dir>
#
# It writes Adelite-<version>-macos-arm64.dmg and .tar.gz into <out dir>.
set -euo pipefail

if [ $# -ne 4 ]; then
    echo "usage: $0 <version> <adelite binary> <libtdjson.dylib> <out dir>" >&2
    exit 2
fi
version=$1 binary=$2 tdjson=$3 out=$4
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "the version is X.Y.Z, not $version" >&2
    exit 2
fi
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../.." && pwd)
icon="$root/crates/desktop/assets/icon/macos.png"
name="Adelite-$version-macos-arm64"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
# What the .dmg shows: Adelite.app, and Applications to drag it to.
disk="$work/disk"
app="$disk/Adelite.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Frameworks" "$app/Contents/Resources" "$out"
out=$(cd "$out" && pwd)

sed "s/@VERSION@/$version/g" "$here/Info.plist" >"$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"
install -m 755 "$binary" "$app/Contents/MacOS/adelite"
# The app looks for TDLib in ../Frameworks beside its executable.
install -m 755 "$tdjson" "$app/Contents/Frameworks/libtdjson.dylib"

# The icon: every size of an iconset the picture is large enough for, so none
# is scaled up.
side=$(sips -g pixelWidth "$icon" | awk '/pixelWidth/ { print $2 }')
iconset="$work/Adelite.iconset"
mkdir "$iconset"
for size in 16 32 128 256 512; do
    if [ "$size" -le "$side" ]; then
        sips -z "$size" "$size" "$icon" --out "$iconset/icon_${size}x${size}.png" >/dev/null
    fi
    if [ $((size * 2)) -le "$side" ]; then
        sips -z $((size * 2)) $((size * 2)) "$icon" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
    fi
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/Adelite.icns"

# Apple silicon runs no unsigned code, so an ad-hoc signature: the library
# first, since the app's own signature seals what is inside it.
codesign --force --sign - "$app/Contents/Frameworks/libtdjson.dylib"
codesign --force --sign - "$app"
codesign --verify --deep --strict "$app"

# The updater's copy. COPYFILE_DISABLE keeps macOS from adding ._ files for
# extended attributes, and ustar is the plainest format to read.
COPYFILE_DISABLE=1 tar --format ustar -czf "$out/$name.tar.gz" -C "$disk" Adelite.app

ln -s /Applications "$disk/Applications"
# hdiutil now and then finds the disk busy on build machines; another try works.
for try in 1 2 3; do
    if hdiutil create -volname Adelite -srcfolder "$disk" -format UDZO -ov "$out/$name.dmg"; then
        break
    fi
    if [ "$try" -eq 3 ]; then
        exit 1
    fi
    sleep 5
done

ls -l "$out/$name.tar.gz" "$out/$name.dmg"
