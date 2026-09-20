#!/bin/sh
set -eu

# Differential gate for the Rust soundemittersystem module. Builds the C++
# module it replaces as an oracle and requires both to agree over a game's
# own sound scripts. No game is launched.
main()
{
ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
[ "$#" -ge 3 ] || { echo "usage: $0 <installed-rust-runtime> <waf-build-dir> <rust-module> [<game-dir>[:<inherited-dir>...]...]" >&2; exit 2; }
RUNTIME=$(CDPATH= cd -- "$1" && pwd)
BUILD=$(CDPATH= cd -- "$2" && pwd)
RUST_MODULE=$(CDPATH= cd -- "$(dirname "$3")" && pwd)/$(basename "$3")
shift 3
[ "$(uname -s)" = Darwin ] || { echo "soundemitter gate currently requires macOS" >&2; exit 2; }
TEST_DIR=$(mktemp -d /tmp/source-soundemitter.XXXXXX)

# The factory, and the target of the thunk that catches the indirect-result
# register for AddWaveName, which has to be a named symbol for the branch to
# resolve and which the linker will not hide beside Rust's export list.
EXPORTS=$(nm -gU "$RUST_MODULE" | awk '{print $NF}' | sort | tr '\n' ' ')
[ "$EXPORTS" = "_CreateInterface _source_add_wave_name " ] || { echo "unexpected exports in $RUST_MODULE: $EXPORTS" >&2; exit 1; }
if otool -L "$RUST_MODULE" | grep -E 'libc\+\+|libtier0|libvstdlib'; then
  echo "Rust soundemittersystem links C++ libraries" >&2
  exit 1
fi
if nm -u "$RUST_MODULE" | grep -E '___cxa|___gxx|__Zn[wa]|__Zd[la]'; then
  echo "Rust soundemittersystem imports the C++ runtime" >&2
  exit 1
fi

DEFINES="-DPOSIX=1 -D_POSIX=1 -DOSX=1 -D_OSX=1 -DGNUC -DPLATFORM_64BITS=1 -DPLATFORM_POSIX=1 -DNO_HOOK_MALLOC -DNO_MEMOVERRIDE_NEW_DELETE=1 -D_DLL_EXT=.dylib -DSOUNDEMITTERSYSTEM_EXPORTS=1 -D_WINDOWS=1"
INCLUDES="-I$ROOT/soundemittersystem -I$ROOT/public -I$ROOT/public/tier0 -I$ROOT/public/tier1 -I$ROOT/game/shared -I$ROOT/common"
# dyld resolves a leaf name in DYLD_LIBRARY_PATH ahead of the path it was
# given, so neither module under test may keep the installed module's name.
NATIVE_MODULE="$TEST_DIR/libsoundemittersystem_native.dylib"
cp "$RUST_MODULE" "$TEST_DIR/libsoundemittersystem_rust.dylib"
RUST_MODULE="$TEST_DIR/libsoundemittersystem_rust.dylib"
# shellcheck disable=SC2086
"${CXX:-clang++}" -shared -fPIC -std=c++11 -w -O1 -DNDEBUG $DEFINES $INCLUDES \
  "$ROOT/soundemittersystem/soundemittersystembase.cpp" \
  "$ROOT/public/SoundParametersInternal.cpp" \
  "$ROOT/game/shared/interval.cpp" \
  "$BUILD/tier1/libtier1.a" \
  -L"$RUNTIME/bin" -ltier0 -lvstdlib -liconv -Wl,-rpath,"$RUNTIME/bin" \
  -o "$NATIVE_MODULE"
# shellcheck disable=SC2086
"${CXX:-clang++}" -std=c++11 -UNDEBUG $DEFINES $INCLUDES -I"$ROOT/appframework" \
  -Wno-inconsistent-missing-override \
  "$ROOT/rust/tests/soundemitter_differential.cpp" \
  -L"$RUNTIME/bin" -ltier0 -lvstdlib -lrust_engine_bridge -Wl,-rpath,"$RUNTIME/bin" \
  -o "$TEST_DIR/differential"
export DYLD_LIBRARY_PATH="$RUNTIME/bin"
FILESYSTEM="$RUNTIME/bin/libfilesystem_stdio.dylib"

for GAME in "$@"; do
  echo "== $GAME"
  "$TEST_DIR/differential" "$FILESYSTEM" "$GAME" "$NATIVE_MODULE" "$RUST_MODULE"
done
rm -rf "$TEST_DIR"
}

# Parse exit together with the invocation, so success never resumes reading a
# potentially edited script at the old byte offset.
main "$@"; exit
