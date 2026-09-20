#!/bin/sh
set -eu

# Differential gate for the Rust steam_api module. Builds the C++ module it
# replaces as an oracle and requires both to answer identically, once through
# dlopen and once through the linker, the way the engine uses it. No game is
# launched.
main()
{
ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
[ "$#" -ge 1 ] || { echo "usage: $0 <rust-module> [<waf-build-dir>]" >&2; exit 2; }
RUST_MODULE=$(CDPATH= cd -- "$(dirname "$1")" && pwd)/$(basename "$1")
[ "$(uname -s)" = Darwin ] || { echo "steam_api gate currently requires macOS" >&2; exit 2; }
TEST_DIR=$(mktemp -d /tmp/source-steam-api.XXXXXX)

# The module is flat C, so its whole export set is the contract: the eight
# subprojects that link it resolve these names and nothing else.
EXPECTED_EXPORTS=45
EXPORTS=$(nm -gU "$RUST_MODULE" | awk '{print $NF}' | sort)
COUNT=$(printf '%s\n' "$EXPORTS" | wc -l | tr -d ' ')
[ "$COUNT" = "$EXPECTED_EXPORTS" ] || { echo "expected $EXPECTED_EXPORTS exports in $RUST_MODULE, found $COUNT" >&2; exit 1; }
if otool -L "$RUST_MODULE" | tail -n +2 | grep -E 'libc\+\+|libtier0|libvstdlib'; then
  echo "Rust steam_api links C++ libraries" >&2
  exit 1
fi
if nm -u "$RUST_MODULE" | grep -E '___cxa|___gxx|__Zn[wa]|__Zd[la]'; then
  echo "Rust steam_api imports the C++ runtime" >&2
  exit 1
fi

DEFINES="-DPOSIX=1 -D_POSIX=1 -DOSX=1 -D_OSX=1 -DGNUC -DPLATFORM_64BITS=1 -DPLATFORM_POSIX=1"
INCLUDES="-I$ROOT/public -I$ROOT/public/tier0"
CXX_TOOL="${CXX:-clang++}"

# The oracle. Built from the module's own translation unit with its own flags,
# and given the same @rpath identity Cargo gives the Rust one, so that the two
# differ in nothing but which compiler produced them.
NATIVE_DIR="$TEST_DIR/native"
RUST_DIR="$TEST_DIR/rust"
mkdir -p "$NATIVE_DIR" "$RUST_DIR"
# shellcheck disable=SC2086
"$CXX_TOOL" -shared -fPIC -std=c++11 -w -O1 -DNDEBUG $DEFINES $INCLUDES \
  -Wl,-install_name,@rpath/libsteam_api.dylib \
  "$ROOT/stub_steam/steam_api.cpp" -o "$NATIVE_DIR/libsteam_api.dylib"
cp "$RUST_MODULE" "$RUST_DIR/libsteam_api.dylib"

# Both modules export the same forty-five names and now share a leaf name, so
# the differential harness reaches them by path and asserts with dladdr that each
# symbol came from the library it asked. Give it the two paths directly.
# shellcheck disable=SC2086
"$CXX_TOOL" -std=c++11 -UNDEBUG -w $DEFINES $INCLUDES \
  "$ROOT/rust/tests/steam_api_differential.cpp" -o "$TEST_DIR/differential"
echo "== dlopen: every export, both modules, same process"
"$TEST_DIR/differential" "$NATIVE_DIR/libsteam_api.dylib" "$RUST_DIR/libsteam_api.dylib"

# The linked gate. Each module gets its own directory and its own copy of the
# program, because they share a leaf name and DYLD_LIBRARY_PATH resolves by leaf
# name: separate directories are what keeps the two runs apart. No -rpath is
# passed, so what has to find the library at run time is DYLD_LIBRARY_PATH
# alone, exactly as in the game.
for VARIANT in native rust; do
  # shellcheck disable=SC2086
  "$CXX_TOOL" -std=c++11 -UNDEBUG -w $DEFINES $INCLUDES \
    "$ROOT/rust/tests/steam_api_link.cpp" \
    -L"$TEST_DIR/$VARIANT" -lsteam_api -o "$TEST_DIR/$VARIANT/link"
  if otool -l "$TEST_DIR/$VARIANT/link" | grep -q LC_RPATH; then
    echo "the linked gate carries an LC_RPATH, so it would not prove the game's lookup" >&2
    exit 1
  fi
  ( cd "$TEST_DIR/$VARIANT" && DYLD_LIBRARY_PATH="$TEST_DIR/$VARIANT" ./link ) > "$TEST_DIR/$VARIANT.out"
done

echo "== linked: real Steamworks prototypes, arguments and all"
if ! diff -u "$TEST_DIR/native.out" "$TEST_DIR/rust.out"; then
  echo "the linked transcripts differ" >&2
  exit 1
fi
sed 's/^/   /' "$TEST_DIR/rust.out"
echo "transcripts identical over $(wc -l < "$TEST_DIR/rust.out" | tr -d ' ') lines"

rm -rf "$TEST_DIR"
echo "PASS: Rust steam_api matches the C++ module it replaces"
}

# Parse exit together with the invocation, so success never resumes reading a
# potentially edited script at the old byte offset.
main "$@"; exit
