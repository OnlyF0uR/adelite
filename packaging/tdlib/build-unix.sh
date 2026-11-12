#!/usr/bin/env bash
# Builds TDLib's JSON interface (libtdjson.dylib on macOS, libtdjson.so on
# Linux) from Telegram's source, the way https://tdlib.github.io/td/build.html
# describes, and copies it into vendor/tdlib/, where the app looks for it.
#
# OpenSSL is linked into the library rather than loaded beside it: people's
# Macs have no Homebrew OpenSSL, and Linux distributions ship different ones.
# zlib comes from the system, which every Mac and Linux has.
#
# Needs: git, CMake, a C++ compiler, gperf, zlib, and OpenSSL with its static
# libraries.
#   macOS (Apple silicon, Xcode's command line tools):
#     brew install gperf cmake openssl@3
#   Debian, Ubuntu:
#     sudo apt-get install gperf cmake g++ libssl-dev zlib1g-dev
#
#   bash packaging/tdlib/build-unix.sh [jobs] [commit]
#
# jobs: compilers run at once (default: one per CPU; TDLib's largest files take
# over 2 GB each). commit: the TDLib commit to build (default: the one pinned in
# COMMIT beside this script, so every build is the same library).
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../.." && pwd)
td="$root/vendor/td"
build="$td/build"
out="$root/vendor/tdlib"
jobs=${1:-$(getconf _NPROCESSORS_ONLN)}
ref=${2:-$(head -n 1 "$here/COMMIT" | tr -d '[:space:]')}

if [[ ! "$jobs" =~ ^[1-9][0-9]*$ ]]; then
    echo "usage: $0 [jobs] [commit]" >&2
    exit 2
fi

flags=(-DCMAKE_BUILD_TYPE=Release -DOPENSSL_USE_STATIC_LIBS=TRUE)
case "$(uname -s)" in
    Darwin)
        lib=libtdjson.dylib
        # Apple silicon, from the oldest macOS the app supports.
        export MACOSX_DEPLOYMENT_TARGET=11.0
        openssl=${OPENSSL_ROOT_DIR:-$(brew --prefix openssl@3)}
        if [ ! -f "$openssl/lib/libcrypto.a" ]; then
            echo "no static OpenSSL in $openssl: brew install openssl@3" >&2
            exit 1
        fi
        flags+=(-DCMAKE_OSX_ARCHITECTURES=arm64 -DCMAKE_OSX_DEPLOYMENT_TARGET=11.0 "-DOPENSSL_ROOT_DIR=$openssl")
        ;;
    Linux)
        lib=libtdjson.so
        ;;
    *)
        echo "this is for macOS and Linux; on Windows, run build-windows.ps1" >&2
        exit 1
        ;;
esac

step() { echo "== $*"; }

if [ ! -d "$td/.git" ]; then
    step "clone TDLib"
    git clone https://github.com/tdlib/td.git "$td"
fi
step "check out $ref"
git -C "$td" fetch origin
git -C "$td" checkout --detach "$ref"

step "configure TDLib"
# CMake remembers what an earlier configure found (a shared OpenSSL, say):
# start from these flags every time. What was compiled stays.
rm -f "$build/CMakeCache.txt"
cmake -S "$td" -B "$build" "${flags[@]}"

step "build tdjson"
cmake --build "$build" --target tdjson --parallel "$jobs"

# The build names the library by TDLib's version (libtdjson.so.1.8.70,
# libtdjson.1.8.70.dylib) behind a symlink; the app looks for the plain name,
# so the file itself goes under it.
mkdir -p "$out"
rm -f "$out/$lib"
cp -L "$build/$lib" "$out/$lib"

step "check that OpenSSL is inside"
if [ "$lib" = libtdjson.dylib ]; then
    # The first line is the file's own name.
    deps=$(otool -L "$out/$lib" | tail -n +2)
    # Nothing from Homebrew either: people's Macs don't have it.
    bad=$(grep -E 'lib(ssl|crypto)[.-]|/opt/homebrew/|/usr/local/' <<<"$deps" || true)
else
    deps=$(ldd "$out/$lib")
    bad=$(grep -E 'lib(ssl|crypto)[.-]' <<<"$deps" || true)
fi
echo "$deps"
if [ -n "$bad" ]; then
    echo "error: $lib loads these at run time, which people's machines may not have:" >&2
    echo "$bad" >&2
    exit 1
fi

echo "== done: $out"
ls -l "$out"
