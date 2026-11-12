#!/usr/bin/env bash
# Packs a Linux release build and TDLib as a .tar.gz with one folder,
# adelite/: the binary, libtdjson.so beside it (where the app looks first),
# and a desktop entry with its icon.
#
#   bash packaging/linux/package.sh <version> <adelite binary> <libtdjson.so> <out dir>
#
# It writes Adelite-<version>-linux-x64.tar.gz into <out dir>.
set -euo pipefail

if [ $# -ne 4 ]; then
    echo "usage: $0 <version> <adelite binary> <libtdjson.so> <out dir>" >&2
    exit 2
fi
version=$1 binary=$2 tdjson=$3 out=$4
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "the version is X.Y.Z, not $version" >&2
    exit 2
fi
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../.." && pwd)
mkdir -p "$out"
out=$(cd "$out" && pwd)

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
dir="$work/adelite"
mkdir "$dir"
install -m 755 "$binary" "$dir/adelite"
install -m 755 "$tdjson" "$dir/libtdjson.so"
install -m 644 "$here/adelite.desktop" "$dir/adelite.desktop"
install -m 644 "$root/crates/desktop/assets/icon/256.png" "$dir/adelite.png"

# Owned by root in the archive, not by the build machine's user, for whoever
# unpacks it with sudo; ustar is the plainest format to read.
tar --format=ustar --owner=0 --group=0 --numeric-owner \
    -czf "$out/Adelite-$version-linux-x64.tar.gz" -C "$work" adelite

ls -l "$out/Adelite-$version-linux-x64.tar.gz"
