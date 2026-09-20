#!/bin/sh
set -eu

# Differential gate for the Rust scenefilecache module. Builds the C++ module
# it replaces as an oracle, then requires both to give identical answers over a
# synthetic scene image and over every shipped archive named on the command
# line. No game is launched.
main()
{
ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
[ "$#" -ge 3 ] || { echo "usage: $0 <installed-rust-runtime> <waf-build-dir> <rust-module> [<archive_dir.vpk>...]" >&2; exit 2; }
RUNTIME=$(CDPATH= cd -- "$1" && pwd)
BUILD=$(CDPATH= cd -- "$2" && pwd)
RUST_MODULE=$(CDPATH= cd -- "$(dirname "$3")" && pwd)/$(basename "$3")
shift 3
[ "$(uname -s)" = Darwin ] || { echo "scenefilecache gate currently requires macOS" >&2; exit 2; }
TEST_DIR=$(mktemp -d /tmp/source-scenefilecache.XXXXXX)

# The module under test must be the Rust one: a factory, nothing else exported,
# and no C++ runtime or engine library behind it.
[ "$(nm -gU "$RUST_MODULE" | awk '{print $NF}')" = "_CreateInterface" ] || { echo "unexpected exports in $RUST_MODULE" >&2; exit 1; }
if otool -L "$RUST_MODULE" | grep -E 'libc\+\+|libtier0|libvstdlib'; then
  echo "Rust scenefilecache links C++ libraries" >&2
  exit 1
fi
if nm -u "$RUST_MODULE" | grep -E '___cxa|___gxx|__Zn[wa]|__Zd[la]'; then
  echo "Rust scenefilecache imports the C++ runtime" >&2
  exit 1
fi

DEFINES="-DPOSIX=1 -D_POSIX=1 -DOSX=1 -D_OSX=1 -DGNUC -DPLATFORM_64BITS=1 -DPLATFORM_POSIX=1 -DNO_HOOK_MALLOC -DNO_MEMOVERRIDE_NEW_DELETE=1 -D_DLL_EXT=.dylib"
# DYLD_LIBRARY_PATH below makes dyld prefer a library of the same leaf name in
# the runtime over the path it was asked for, so neither module under test may
# keep the installed module's name.
NATIVE_MODULE="$TEST_DIR/libscenefilecache_native.dylib"
cp "$RUST_MODULE" "$TEST_DIR/libscenefilecache_rust.dylib"
RUST_MODULE="$TEST_DIR/libscenefilecache_rust.dylib"
# shellcheck disable=SC2086
"${CXX:-clang++}" -shared -fPIC -std=c++11 -w -O1 -DNDEBUG $DEFINES \
  -I"$ROOT/public" -I"$ROOT/public/tier0" -I"$ROOT/public/tier1" -I"$ROOT/game/shared" -I"$ROOT/common" \
  "$ROOT/scenefilecache/SceneFileCache.cpp" "$BUILD/tier1/libtier1.a" \
  -L"$RUNTIME/bin" -ltier0 -lvstdlib -liconv -Wl,-rpath,"$RUNTIME/bin" \
  -o "$NATIVE_MODULE"
# Link the bridge's exported ABI, which owns the live filesystem context.
# shellcheck disable=SC2086
"${CXX:-clang++}" -std=c++11 -UNDEBUG $DEFINES -Wno-inconsistent-missing-override \
  -I"$ROOT/public" -I"$ROOT/public/tier0" -I"$ROOT/public/tier1" -I"$ROOT/appframework" \
  "$ROOT/rust/tests/scenefilecache_differential.cpp" -L"$RUNTIME/bin" \
  -ltier0 -lrust_engine_bridge -Wl,-rpath,"$RUNTIME/bin" \
  -o "$TEST_DIR/differential"
"${CC:-clang}" -shared -fPIC -D_7ZIP_ST \
  "$ROOT/utils/lzma/C/LzmaLib.c" "$ROOT/utils/lzma/C/LzmaEnc.c" \
  "$ROOT/utils/lzma/C/LzmaDec.c" "$ROOT/utils/lzma/C/LzFind.c" \
  "$ROOT/utils/lzma/C/Alloc.c" -o "$TEST_DIR/lzma_encoder.dylib"
export DYLD_LIBRARY_PATH="$RUNTIME/bin"
FILESYSTEM="$RUNTIME/bin/libfilesystem_stdio.dylib"
FIXTURE="$ROOT/rust/tests/scenefilecache_fixture.py"

SCENES=$(python3 "$FIXTURE" synthetic "$TEST_DIR/synthetic" "$TEST_DIR/lzma_encoder.dylib")
echo "== synthetic image, $SCENES scenes"
"$TEST_DIR/differential" "$FILESYSTEM" "$TEST_DIR/synthetic/game" \
  "$NATIVE_MODULE" "$RUST_MODULE" "$TEST_DIR/synthetic/names.txt" "$SCENES"

echo "== image with the wrong version"
STATUS=0
"$TEST_DIR/differential" bad-image "$FILESYSTEM" "$TEST_DIR/synthetic/bad" "$RUST_MODULE" \
  > "$TEST_DIR/bad.log" 2>&1 || STATUS=$?
if [ "$STATUS" -ne 1 ] || ! grep -q 'Bad scene image file' "$TEST_DIR/bad.log" || grep -q SURVIVED "$TEST_DIR/bad.log"; then
  cat "$TEST_DIR/bad.log" >&2
  echo "a bad scene image did not raise the engine's fatal error (status $STATUS)" >&2
  exit 1
fi
grep 'Bad scene image file' "$TEST_DIR/bad.log"

# The same again for a process that did not load tier0 from the directory dyld
# would search for it, where asking for the library by name finds nothing and
# the module has to recognise it among the images already loaded. A miss here
# would turn the fatal error into a line on stderr and a game with no scenes.
echo "== image with the wrong version, tier0 loaded from elsewhere"
mkdir "$TEST_DIR/elsewhere"
cp "$RUNTIME/bin/libtier0.dylib" "$RUNTIME/bin/libvstdlib.dylib" "$RUNTIME/bin/libfilesystem_stdio.dylib" \
  "$RUNTIME/bin/librust_engine_bridge.dylib" "$RUNTIME/bin/libsource_abi.dylib" "$TEST_DIR/elsewhere/"
STATUS=0
env -u DYLD_LIBRARY_PATH DYLD_INSERT_LIBRARIES="$TEST_DIR/elsewhere/libtier0.dylib" DYLD_FALLBACK_LIBRARY_PATH="$RUNTIME/bin" \
  "$TEST_DIR/differential" bad-image "$TEST_DIR/elsewhere/libfilesystem_stdio.dylib" "$TEST_DIR/synthetic/bad" "$RUST_MODULE" \
  > "$TEST_DIR/bad-elsewhere.log" 2>&1 || STATUS=$?
if grep -q SURVIVED "$TEST_DIR/bad-elsewhere.log"; then
  cat "$TEST_DIR/bad-elsewhere.log" >&2
  echo "the module did not find tier0 among the loaded images" >&2
  exit 1
fi
echo "status $STATUS: $(grep -c 'Bad scene image file' "$TEST_DIR/bad-elsewhere.log") fatal error line(s)"

VPK_TOOL="$BUILD/cargo-target/release/source-vpk"
INDEX=0
for ARCHIVE in "$@"; do
  INDEX=$((INDEX + 1))
  CONTENT="$TEST_DIR/content-$INDEX"
  mkdir -p "$CONTENT/game/scenes"
  "$VPK_TOOL" extract-file "$ARCHIVE" scenes/scenes.image "$CONTENT/game/scenes/scenes.image"
  # The image files its scenes by checksum alone, so the names to ask for
  # come from every archive beside it: response rules, maps and scripts.
  # An episode's image repeats the base game's scenes, whose names are in the
  # base game's archives, so every archive directory named is scraped for each.
  NAMES=$(for SOURCE in "$@"; do ls "$(dirname "$SOURCE")"/*.vpk; done | sort -u | tr '\n' '\0' \
    | xargs -0 python3 "$FIXTURE" names "$CONTENT/names.txt")
  # Every scene, named or not, has to decode to the size it declares.
  echo "== $ARCHIVE: $("$BUILD/cargo-target/release/examples/walk_cache" "$CONTENT/game/scenes/scenes.image")"
  echo "== $ARCHIVE, $NAMES candidate names"
  "$TEST_DIR/differential" "$FILESYSTEM" "$CONTENT/game" \
    "$NATIVE_MODULE" "$RUST_MODULE" "$CONTENT/names.txt" 100
done
rm -rf "$TEST_DIR"
}

# Parse exit together with the invocation, so success never resumes reading a
# potentially edited script at the old byte offset.
main "$@"; exit
